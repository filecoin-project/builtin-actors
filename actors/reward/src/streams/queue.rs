//! The pending-write queue: the SWA timelock, admission, cancellation, and application.
//!
//! Field order and enum discriminants are wire format for the types declared here.
//!
//! Every SWA write lands here first as a `PendingWrite` containing the op, its encoded params,
//! and the  epoch it becomes due. Each one sits in a [`Slot`]. Per-stream ops key by `(id, op)`;
//! the two weight ops are schedule-wide and key by `op` alone (i.e. null id). One write per slot,
//! and an occupied slot means a new write for that slot will be rejected, so changing your mind
//! means a cancel plus requeue and the timelock starts again. `CancelPending` has to name the slot
//! being cancelled the same way the queue keys it. For cancellation a mismatched id/op is an error,
//! an empty slot is a no-op, and `StepWeightRecords` can't be cancelled at all (the discretionary
//! `SetWeightRecords` can be).
//!
//! Three epochs matter for an entry. The queue epoch is when the SWA called. The effective epoch
//! is queue plus `swa_timelock_epochs`, except for `RegisterStream`, which brings its own
//! `activation_epoch` and just has to be at or past that floor. The apply epoch is whenever the
//! next stream-engine method runs at or after that, `AwardBlockReward` included, because every
//! one of them applies due entries before doing its own thing. The queue is sorted by effective
//! epoch with ties in arrival order, so the head tells you whether anything is due. The
//! objection window is `[queue, effective)`, and a due entry applies before a same-epoch cancel.
//!
//! One rule drives most of the logic in here: *cancellation is unconditional*. A compromised SWA
//! must not be able to make a bad write uncancellable by queueing something that depends on it,
//! and both governance multisigs may relay the same objection. So an admitted write can lose a
//! prerequisite after admission, application has to be able to drop it, and admission has to
//! validate against the state _as it will be at application_ rather than today's, since two
//! writes, each fine on their own, can sum past one together. That's why both
//! `validate_new_pending` and `apply_due_writes` replay the queue in effective order.
//!
//! The admission rule: *W is admitted iff, applying the queue in effective order, W applies in
//! the state its predecessors produce, and every write that applied before W's admission still
//! applies after it*. The second clause is the subset check in `validate_new_pending`. It's
//! there because a `RegisterStream` can have a far-future activation, so a `SetWeightRecords`
//! admitted later but effective earlier applies first and could eat the headroom the
//! registration was admitted on. Without it, admission would be a second way to strand a write,
//! and cancellation is meant to be the only one.
//!
//! What each op needs at its effective epoch:
//!
//! | op | needs at its effective epoch |
//! |---|---|
//! | `SetWeightRecords`, `StepWeightRecords` | every target id live; envelope holds from the effective epoch |
//! | `RegisterStream` | id not live or tombstoned; stream table below `MAX_STREAMS`; if implicit, no other implicit; envelope holds from activation |
//! | `RemoveStream` | id live |
//! | `SetDistribution` | id live and explicit |
//!
//! And which earlier writes provide one of those, so cancelling them takes it away:
//!
//! | earlier write | provides, for a later write |
//! |---|---|
//! | `RegisterStream(id)` | existence of `id` for a later `Set`, `Step`, `Remove` or `SetDistribution` on it |
//! | `RemoveStream(id)` | a table slot, the implicit slot, and envelope headroom for a later `Register`, `Set` or `Step` |
//! | `SetWeightRecords` (a decrease) | envelope headroom for a later `Set`, `Step` or `Register` |
//!
//! So there's three providers and four things they can provide (existence, table room, the implicit
//! slot, envelope headroom). `StepWeightRecords` can't be cancelled and `SetDistribution` provides
//! nothing, so neither strands anything. The envelope is the piecewise-linear sum check over in
//! `weights`, and it's the only one with (awkward) arithmetic in it.
//!
//! At application, due entries go in queue order, each validated from its own effective epoch
//! rather than the current one, so a null round can't change which ones apply. A well-formed
//! entry that's lost its prerequisite is dropped with a `write-dropped` event and we carry on;
//! the installed config is untouched. A payload that won't decode, or a queue that isn't
//! canonical, is corruption and explicit methods abort and the award pays gas only.
//!
//! If the persisted weight records or envelope are already invalid, due entries still only
//! apply when their result validates, and new ones are rejected, with one exception: a
//! `SetWeightRecords` that repairs things without stranding a still-valid entry. It gets the
//! ordinary timelock like anything else.
//!
//! FIP-0118 2.4.7 has the timelock, the three epochs, slots and cancellability; 2.4.8 has details
//! about what admission proves, stranding, and repair.

use std::collections::BTreeSet;

use anyhow::{Result, ensure};
use fvm_ipld_encoding::RawBytes;
use fvm_ipld_encoding::repr::*;
use fvm_ipld_encoding::tuple::*;
use fvm_shared::address::Address;
use fvm_shared::clock::ChainEpoch;
use fvm_shared::econ::TokenAmount;
use num_traits::Zero;

use super::distribution::{
    DistributionInit, ExplicitDistribution, RecipientTable, fold, validate_distribution_init,
    validate_id_address,
};
use super::invariants::{
    validate_mutation_state, validate_stream_configuration,
    validate_stream_configuration_without_weights, validate_tombstone_capacity,
};
use super::weights::{
    WeightRecord, WeightRecordUpdate, WeightRecordsPayload, validate_weight_record,
    validate_weight_schedule, validate_weight_updates,
};
use super::{MAX_PENDING_WRITES, Stream, StreamAccrual, StreamId, StreamsState, Tombstone};

const EMPTY_TUPLE_CBOR: &[u8] = &[0x80];

/// Stable operation tag persisted in `PendingWrite` and exposed by `CancelPending`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize_repr, Deserialize_repr)]
#[repr(u8)]
pub enum PendingWriteOp {
    SetWeightRecords = 0,
    /// Gate-originated weight update; unlike other operations, it cannot be cancelled.
    StepWeightRecords = 1,
    RegisterStream = 2,
    RemoveStream = 3,
    SetDistribution = 4,
}

impl PendingWriteOp {
    /// Schedule-wide operations act on the whole weight schedule and carry no stream ID.
    fn is_schedule_wide(self) -> bool {
        matches!(self, PendingWriteOp::SetWeightRecords | PendingWriteOp::StepWeightRecords)
    }
}

/// The queue position that a single pending write occupies (one write per slot).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum Slot {
    PerStream(StreamId, PendingWriteOp),
    ScheduleWide(PendingWriteOp),
}

impl Slot {
    /// The slot that an operation on `id` occupies, rejecting a mismatched ID and operation.
    fn for_target(id: Option<StreamId>, op: PendingWriteOp) -> Result<Slot> {
        match (op.is_schedule_wide(), id) {
            (true, None) => Ok(Slot::ScheduleWide(op)),
            (true, Some(_)) => Err(anyhow::anyhow!("schedule-wide call has a stream ID")),
            (false, Some(id)) => Ok(Slot::PerStream(id, op)),
            (false, None) => Err(anyhow::anyhow!("per-stream call has no stream ID")),
        }
    }

    /// The slot `CancelPending` identifies with `StepWeightRecords` being the one uncancellable
    /// operation.
    pub(crate) fn for_cancel(id: Option<StreamId>, op: PendingWriteOp) -> Result<Slot> {
        ensure!(op != PendingWriteOp::StepWeightRecords, "StepWeightRecords cannot be cancelled");
        Slot::for_target(id, op)
    }
}

/// A deferred SWA operation persisted in `StreamsState`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize_tuple, Deserialize_tuple)]
pub struct PendingWrite {
    /// Per-stream target, or `None` for a schedule-wide call.
    pub id: Option<StreamId>,
    pub op: PendingWriteOp,
    /// Operation-specific CBOR tuple.
    pub payload: RawBytes,
    pub effective_epoch: ChainEpoch,
}

impl PendingWrite {
    /// The slot this write occupies; `Err` for a persisted ID and operation that disagree.
    fn slot(&self) -> Result<Slot> {
        Slot::for_target(self.id, self.op)
    }
}

/// A queued write's operation and arguments, decoded from its payload.
///
/// Every read of a payload produces one of these and every write of a payload comes from one, so
/// the CBOR in `PendingWrite.payload` is confined to [`QueuedCall::decode`] and
/// [`QueuedCall::encode`].
#[derive(Clone, Debug, PartialEq, Eq)]
enum QueuedCall {
    Weights { op: PendingWriteOp, updates: Vec<WeightRecordUpdate> },
    Register { id: StreamId, weight: WeightRecord, distribution: Option<DistributionInit> },
    Remove { id: StreamId },
    SetDistribution { id: StreamId, writer: Address },
}

impl QueuedCall {
    /// Reads a queue entry's payload, rejecting one that disagrees with the entry's ID and
    /// operation or does not hold that operation's canonical tuple.
    fn decode(write: &PendingWrite) -> Result<QueuedCall> {
        Ok(match Slot::for_target(write.id, write.op)? {
            Slot::ScheduleWide(op) => {
                let payload: WeightRecordsPayload = write.payload.deserialize()?;
                validate_weight_updates(&payload.updates)?;
                QueuedCall::Weights { op, updates: payload.updates }
            }
            Slot::PerStream(id, PendingWriteOp::RegisterStream) => {
                let payload: RegisterStreamPayload = write.payload.deserialize()?;
                validate_weight_record(&payload.weight)?;
                validate_distribution_init(&payload.distribution)?;
                QueuedCall::Register {
                    id,
                    weight: payload.weight,
                    distribution: payload.distribution,
                }
            }
            Slot::PerStream(id, PendingWriteOp::RemoveStream) => {
                ensure!(
                    write.payload.bytes() == EMPTY_TUPLE_CBOR,
                    "RemoveStream payload is not an empty tuple"
                );
                QueuedCall::Remove { id }
            }
            Slot::PerStream(id, PendingWriteOp::SetDistribution) => {
                let payload: SetDistributionPayload = write.payload.deserialize()?;
                validate_id_address(&payload.writer, "distribution writer")?;
                QueuedCall::SetDistribution { id, writer: payload.writer }
            }
            // Slot::for_target puts both weight operations in the schedule-wide arm.
            Slot::PerStream(_, op) => {
                return Err(anyhow::anyhow!("schedule-wide call {op:?} has a stream ID"));
            }
        })
    }

    /// The payload bytes a queue entry carries for this call.
    fn encode(&self) -> Result<RawBytes> {
        Ok(match self {
            QueuedCall::Weights { updates, .. } => {
                RawBytes::serialize(&WeightRecordsPayload { updates: updates.clone() })?
            }
            QueuedCall::Register { weight, distribution, .. } => {
                RawBytes::serialize(&RegisterStreamPayload {
                    weight: weight.clone(),
                    distribution: distribution.clone(),
                })?
            }
            QueuedCall::Remove { .. } => RawBytes::new(EMPTY_TUPLE_CBOR.to_vec()),
            QueuedCall::SetDistribution { writer, .. } => {
                RawBytes::serialize(&SetDistributionPayload { writer: *writer })?
            }
        })
    }

    fn slot(&self) -> Slot {
        match self {
            QueuedCall::Weights { op, .. } => Slot::ScheduleWide(*op),
            QueuedCall::Register { id, .. } => Slot::PerStream(*id, PendingWriteOp::RegisterStream),
            QueuedCall::Remove { id } => Slot::PerStream(*id, PendingWriteOp::RemoveStream),
            QueuedCall::SetDistribution { id, .. } => {
                Slot::PerStream(*id, PendingWriteOp::SetDistribution)
            }
        }
    }

    /// The queue entry carrying this call, due at `effective_epoch`.
    fn queue_entry(&self, effective_epoch: ChainEpoch) -> Result<PendingWrite> {
        let (id, op) = match self.slot() {
            Slot::PerStream(id, op) => (Some(id), op),
            Slot::ScheduleWide(op) => (None, op),
        };
        Ok(PendingWrite { id, op, payload: self.encode()?, effective_epoch })
    }
}

/// The tuple stored in `PendingWrite.payload` for `RegisterStream`.
///
/// This is the deferred call's own payload, not the `RegisterStream` method parameter tuple.
#[derive(Clone, Debug, PartialEq, Eq, Serialize_tuple, Deserialize_tuple)]
pub struct RegisterStreamPayload {
    pub weight: WeightRecord,
    pub distribution: Option<DistributionInit>,
}

/// The tuple stored in `PendingWrite.payload` for `SetDistribution`.
///
/// This is the deferred call's own payload, not the `SetDistribution` method parameter tuple.
#[derive(Clone, Debug, PartialEq, Eq, Serialize_tuple, Deserialize_tuple)]
pub struct SetDistributionPayload {
    pub writer: Address,
}

/// The effects of applying due writes, for the actor layer to settle after the transaction.
///
/// This and the effect types below cross Rust call boundaries only so don't need to be encodable.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ApplyResult {
    pub fold_dust: TokenAmount,
    /// Successful writes for actor-layer events after a committed application. Admission-only
    /// projections discard them.
    pub applied: Vec<PendingWrite>,
    /// Removed writes for actor-layer events after a committed application. Admission-only
    /// projections discard them.
    pub dropped: Vec<PendingWrite>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct ProjectedStreams {
    pub streams: StreamsState,
    pub accruals: Vec<StreamAccrual>,
    pub apply_result: ApplyResult,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct CancelResult {
    pub apply_result: ApplyResult,
    /// Cancelled write for actor-layer events after a committed removal.
    pub removed: Option<PendingWrite>,
}

/// Queues a weight batch at the timelock boundary after validating all projected writes.
pub(crate) fn queue_weight_records(
    streams: &mut StreamsState,
    current_epoch: ChainEpoch,
    timelock_epochs: ChainEpoch,
    op: PendingWriteOp,
    updates: &[WeightRecordUpdate],
) -> Result<PendingWrite> {
    ensure!(op.is_schedule_wide(), "invalid weight-record operation {op:?}");
    let effective_epoch = timelock_epoch(current_epoch, timelock_epochs)?;
    let mut updates = updates.to_vec();
    updates.sort_by_key(|update| update.id);
    validate_weight_updates(&updates)?;
    let queued = QueuedCall::Weights { op, updates }.queue_entry(effective_epoch)?;

    let mut proposed = streams.clone();
    ensure_slot_available(&proposed, Slot::ScheduleWide(op))?;
    proposed.pending_writes.push(queued.clone());
    sort_pending(&mut proposed.pending_writes);
    validate_new_pending(streams, &proposed, current_epoch, Slot::ScheduleWide(op))?;

    *streams = proposed;
    Ok(queued)
}

/// Queues registration at an activation epoch no earlier than the timelock boundary.
pub(crate) fn queue_register_stream(
    streams: &mut StreamsState,
    current_epoch: ChainEpoch,
    timelock_epochs: ChainEpoch,
    stream: Stream,
    activation_epoch: ChainEpoch,
) -> Result<PendingWrite> {
    ensure!(stream.id != 0, "stream ID 0 is reserved");
    validate_weight_record(&stream.weight)?;
    if let Some(distribution) = stream.explicit()
        && (!distribution.payable.is_empty() || !distribution.claimed_period.is_empty())
    {
        return Err(anyhow::anyhow!("new stream has pre-existing accounting state"));
    }
    let mut distribution = stream.distribution.map(|distribution| DistributionInit {
        writer: distribution.writer,
        shares: distribution.shares,
    });
    if let Some(distribution) = &mut distribution {
        distribution.shares.sort_by_key(|row| row.recipient);
    }
    validate_distribution_init(&distribution)?;
    ensure_stream_id_available(streams, stream.id)?;
    let slot = Slot::PerStream(stream.id, PendingWriteOp::RegisterStream);
    ensure_slot_available(streams, slot)?;

    let earliest = timelock_epoch(current_epoch, timelock_epochs)?;
    ensure!(
        activation_epoch >= earliest,
        "activation epoch {activation_epoch} is before timelock floor {earliest}"
    );
    let queued = QueuedCall::Register { id: stream.id, weight: stream.weight, distribution }
        .queue_entry(activation_epoch)?;

    let mut proposed = streams.clone();
    proposed.pending_writes.push(queued.clone());
    sort_pending(&mut proposed.pending_writes);
    validate_new_pending(streams, &proposed, current_epoch, slot)?;

    *streams = proposed;
    Ok(queued)
}

/// Queues removal; explicit-stream liabilities are settled when the write applies.
pub(crate) fn queue_remove_stream(
    streams: &mut StreamsState,
    current_epoch: ChainEpoch,
    timelock_epochs: ChainEpoch,
    id: StreamId,
) -> Result<PendingWrite> {
    let effective_epoch = timelock_epoch(current_epoch, timelock_epochs)?;
    let slot = Slot::PerStream(id, PendingWriteOp::RemoveStream);
    ensure_slot_available(streams, slot)?;
    let queued = QueuedCall::Remove { id }.queue_entry(effective_epoch)?;

    let mut proposed = streams.clone();
    proposed.pending_writes.push(queued.clone());
    sort_pending(&mut proposed.pending_writes);
    validate_new_pending(streams, &proposed, current_epoch, slot)?;
    validate_tombstone_capacity(&proposed)?;

    *streams = proposed;
    Ok(queued)
}

/// Queues writer replacement; the outgoing period is settled when the write applies.
pub(crate) fn queue_set_distribution(
    streams: &mut StreamsState,
    current_epoch: ChainEpoch,
    timelock_epochs: ChainEpoch,
    id: StreamId,
    writer: Address,
) -> Result<PendingWrite> {
    let effective_epoch = timelock_epoch(current_epoch, timelock_epochs)?;
    let slot = Slot::PerStream(id, PendingWriteOp::SetDistribution);
    ensure_slot_available(streams, slot)?;
    let queued = QueuedCall::SetDistribution { id, writer }.queue_entry(effective_epoch)?;

    let mut proposed = streams.clone();
    proposed.pending_writes.push(queued.clone());
    sort_pending(&mut proposed.pending_writes);
    validate_new_pending(streams, &proposed, current_epoch, slot)?;

    *streams = proposed;
    Ok(queued)
}

/// Empties one queue slot. Cancelling an empty slot is a no-op.
pub(super) fn cancel_pending(streams: &mut StreamsState, slot: Slot) -> Option<PendingWrite> {
    streams
        .pending_writes
        .iter()
        .position(|write| write.slot().is_ok_and(|occupied| occupied == slot))
        .map(|idx| streams.pending_writes.remove(idx))
}

/// Applies writes due through `epoch` before attempting cancellation.
pub(crate) fn apply_due_writes_and_cancel(
    streams: &mut StreamsState,
    accruals: &mut Vec<StreamAccrual>,
    epoch: ChainEpoch,
    slot: Slot,
) -> Result<CancelResult> {
    validate_mutation_state(streams, accruals)?;
    let mut projected = project_due_writes(streams, accruals, epoch)?;
    let removed = cancel_pending(&mut projected.streams, slot);
    *streams = projected.streams;
    *accruals = projected.accruals;
    Ok(CancelResult { apply_result: projected.apply_result, removed })
}

/// Applies every write due through `epoch`, each validated from its own effective epoch just
/// as its admission projected. From invalid weight state, only writes that restore a valid
/// schedule apply and the rest are dropped.
pub(crate) fn apply_due_writes(
    streams: &mut StreamsState,
    accruals: &mut Vec<StreamAccrual>,
    epoch: ChainEpoch,
) -> Result<ApplyResult> {
    validate_mutation_state(streams, accruals)?;
    if streams.pending_writes.first().is_none_or(|write| write.effective_epoch > epoch) {
        return Ok(ApplyResult::default());
    }

    let mut next_streams = streams.clone();
    let mut next_accruals = accruals.clone();

    let due_count = next_streams
        .pending_writes
        .iter()
        .take_while(|write| write.effective_epoch <= epoch)
        .count();
    let due: Vec<_> = next_streams.pending_writes.drain(..due_count).collect();
    let mut fold_dust = TokenAmount::zero();
    let mut applied = Vec::new();
    let mut dropped = Vec::new();

    for write in due {
        let Ok(call) = QueuedCall::decode(&write) else {
            dropped.push(write);
            continue;
        };
        // Records-only first, to decide apply vs drop without touching the accruals. This is the
        // same as admission, but now dealing with whatever cancellation may have removed.
        let mut projected = next_streams.clone();
        let stranded = apply_pending_transition(&mut projected, None, &call)
            .and_then(|_| validate_transition_state(&projected, write.effective_epoch))
            .is_err();
        if stranded {
            dropped.push(write);
            continue;
        }

        let mut candidate_streams = next_streams.clone();
        let mut candidate_accruals = next_accruals.clone();
        let write_dust =
            apply_pending_transition(&mut candidate_streams, Some(&mut candidate_accruals), &call)?;
        validate_transition_state(&candidate_streams, write.effective_epoch)?;
        next_streams = candidate_streams;
        next_accruals = candidate_accruals;
        applied.push(write);
        fold_dust += write_dust;
    }

    *streams = next_streams;
    *accruals = next_accruals;
    Ok(ApplyResult { fold_dust, applied, dropped })
}

/// Projects due writes without mutating the supplied state.
pub(super) fn project_due_writes(
    streams: &StreamsState,
    accruals: &[StreamAccrual],
    epoch: ChainEpoch,
) -> Result<ProjectedStreams> {
    let mut projected_streams = streams.clone();
    let mut projected_accruals = accruals.to_vec();
    let apply_result = apply_due_writes(&mut projected_streams, &mut projected_accruals, epoch)?;
    Ok(ProjectedStreams { streams: projected_streams, accruals: projected_accruals, apply_result })
}

fn timelock_epoch(current_epoch: ChainEpoch, timelock_epochs: ChainEpoch) -> Result<ChainEpoch> {
    ensure!(timelock_epochs >= 0, "timelock is negative");
    current_epoch
        .checked_add(timelock_epochs)
        .ok_or_else(|| anyhow::anyhow!("timelock epoch overflow"))
}

fn ensure_slot_available(streams: &StreamsState, slot: Slot) -> Result<()> {
    ensure!(
        !streams
            .pending_writes
            .iter()
            .any(|write| write.slot().is_ok_and(|occupied| occupied == slot)),
        "pending slot {slot:?} is occupied"
    );
    Ok(())
}

fn ensure_stream_id_available(streams: &StreamsState, id: StreamId) -> Result<()> {
    ensure!(
        !streams.streams.iter().any(|stream| stream.id == id),
        "stream ID {id} is already registered"
    );
    ensure!(
        !streams.tombstones.iter().any(|tombstone| tombstone.id == id),
        "stream ID {id} is tombstoned"
    );
    ensure!(
        !streams
            .pending_writes
            .iter()
            .any(|write| write.id == Some(id) && write.op == PendingWriteOp::RegisterStream),
        "stream ID {id} has a pending registration"
    );
    Ok(())
}

/// Stable sorting preserves insertion order among calls effective at the same epoch.
fn sort_pending(writes: &mut [PendingWrite]) {
    writes.sort_by_key(|write| write.effective_epoch);
}

/// Admits the proposed queue: the new write must apply in the state its predecessors produce,
/// and every write that applied before it must still apply after it.
fn validate_new_pending(
    current: &StreamsState,
    proposed: &StreamsState,
    current_epoch: ChainEpoch,
    new_slot: Slot,
) -> Result<()> {
    // From an invalid schedule, a SetWeightRecords batch is the one admissible repair (2.4.8).
    let repairing = new_slot == Slot::ScheduleWide(PendingWriteOp::SetWeightRecords);
    let accepted_before = match validate_projected_queue(current, current_epoch, None) {
        Ok(accepted) => accepted,
        Err(error) if repairing => {
            validate_projected_queue_recovering(current, current_epoch, None).map_err(|_| error)?
        }
        Err(error) => return Err(error),
    };
    let accepted_after = match validate_projected_queue(proposed, current_epoch, Some(new_slot)) {
        Ok(accepted) => accepted,
        Err(error) if repairing => {
            validate_projected_queue_recovering(proposed, current_epoch, Some(new_slot))
                .map_err(|_| error)?
        }
        Err(error) => return Err(error),
    };
    ensure!(
        accepted_before.is_subset(&accepted_after),
        "new call invalidates an existing pending call"
    );
    Ok(())
}

/// Projects calls in execution order. Calls stranded by cancellation become future drops.
fn validate_projected_queue(
    streams: &StreamsState,
    current_epoch: ChainEpoch,
    required_slot: Option<Slot>,
) -> Result<BTreeSet<Slot>> {
    validate_projected_queue_inner(
        streams,
        current_epoch,
        required_slot,
        false,
        Some(current_epoch),
    )
}

/// Projects a repair from otherwise well-formed state with invalid weight records or envelope.
fn validate_projected_queue_recovering(
    streams: &StreamsState,
    current_epoch: ChainEpoch,
    required_slot: Option<Slot>,
) -> Result<BTreeSet<Slot>> {
    validate_projected_queue_inner(streams, current_epoch, required_slot, true, Some(current_epoch))
}

pub(super) fn validate_projected_queue_inner(
    streams: &StreamsState,
    current_epoch: ChainEpoch,
    required_slot: Option<Slot>,
    allow_invalid_initial_schedule: bool,
    minimum_epoch: Option<ChainEpoch>,
) -> Result<BTreeSet<Slot>> {
    validate_pending_queue(&streams.pending_writes, minimum_epoch)?;
    if allow_invalid_initial_schedule {
        validate_stream_configuration_without_weights(&streams.streams)?;
    } else {
        validate_stream_configuration(&streams.streams)?;
        validate_weight_schedule(&streams.streams, current_epoch)?;
    }
    let mut projected = streams.clone();
    let mut accepted = BTreeSet::new();

    for write in &streams.pending_writes {
        let slot = write.slot()?;
        // Validated from the entry's effective epoch, as at application, so a null round at
        // that epoch cannot change which writes apply.
        let mut candidate = projected.clone();
        let result = QueuedCall::decode(write).and_then(|call| {
            apply_pending_transition(&mut candidate, None, &call)?;
            validate_transition_state(&candidate, write.effective_epoch)
        });
        match result {
            Ok(()) => {
                projected = candidate;
                accepted.insert(slot);
            }
            Err(error) if Some(slot) == required_slot => {
                return Err(anyhow::anyhow!("pending call {slot:?} is invalid: {error}"));
            }
            Err(_) => {}
        }
    }
    Ok(accepted)
}

fn validate_transition_state(streams: &StreamsState, start_epoch: ChainEpoch) -> Result<()> {
    validate_stream_configuration(&streams.streams)?;
    validate_weight_schedule(&streams.streams, start_epoch)?;
    validate_tombstone_capacity(streams)
}

pub(super) fn validate_pending_queue(
    writes: &[PendingWrite],
    minimum_epoch: Option<ChainEpoch>,
) -> Result<()> {
    ensure!(
        writes.is_sorted_by_key(|write| write.effective_epoch),
        "pending writes are not ordered"
    );
    ensure!(
        writes.len() <= MAX_PENDING_WRITES,
        "pending write count {} exceeds maximum {MAX_PENDING_WRITES}",
        writes.len()
    );
    let mut slots = BTreeSet::new();
    for write in writes {
        let slot = QueuedCall::decode(write)?.slot();
        ensure!(slots.insert(slot), "duplicate pending slot {slot:?}");
        if let Some(epoch) = minimum_epoch
            && write.effective_epoch < epoch
        {
            return Err(anyhow::anyhow!("pending write {slot:?} is in the past"));
        }
    }
    Ok(())
}

/// Applies one captured call.
///
/// Without accruals it moves stream records only, which is what a projection needs to answer
/// "would this write apply". With accruals it is the committing form: it also folds the closing
/// period, tombstones what a removal leaves behind, and adds or drops the stream's accrual row.
fn apply_pending_transition(
    state: &mut StreamsState,
    accruals: Option<&mut Vec<StreamAccrual>>,
    call: &QueuedCall,
) -> Result<TokenAmount> {
    match call {
        QueuedCall::Weights { updates, .. } => {
            for update in updates {
                let stream = state
                    .streams
                    .iter_mut()
                    .find(|stream| stream.id == update.id)
                    .ok_or_else(|| anyhow::anyhow!("stream {} not found", update.id))?;
                stream.weight = update.weight.clone();
            }
            Ok(TokenAmount::zero())
        }
        QueuedCall::Register { id, weight, distribution } => {
            let id = *id;
            ensure!(id != 0, "stream ID 0 is reserved");
            ensure!(
                !state.streams.iter().any(|stream| stream.id == id),
                "stream ID {id} is already registered"
            );
            ensure!(
                !state.tombstones.iter().any(|tombstone| tombstone.id == id),
                "stream ID {id} is tombstoned"
            );
            let distribution = distribution.as_ref().map(|distribution| ExplicitDistribution {
                writer: distribution.writer,
                shares: distribution.shares.clone(),
                payable: RecipientTable::default(),
                claimed_period: RecipientTable::default(),
            });
            if distribution.is_some()
                && let Some(accruals) = accruals
            {
                accruals.push(StreamAccrual { id, amount: TokenAmount::zero() });
                accruals.sort_by_key(|row| row.id);
            }
            state.streams.push(Stream { id, weight: weight.clone(), distribution });
            state.streams.sort_by_key(|stream| stream.id);
            Ok(TokenAmount::zero())
        }
        QueuedCall::Remove { id } => {
            let id = *id;
            match accruals {
                Some(accruals) => {
                    remove_stream(&mut state.streams, &mut state.tombstones, accruals, id)
                }
                None => {
                    let idx = state
                        .streams
                        .iter()
                        .position(|stream| stream.id == id)
                        .ok_or_else(|| anyhow::anyhow!("stream {id} not found"))?;
                    state.streams.remove(idx);
                    Ok(TokenAmount::zero())
                }
            }
        }
        QueuedCall::SetDistribution { id, writer } => {
            let id = *id;
            match accruals {
                Some(accruals) => replace_writer(&mut state.streams, accruals, id, *writer),
                None => {
                    let stream = state
                        .streams
                        .iter_mut()
                        .find(|stream| stream.id == id)
                        .ok_or_else(|| anyhow::anyhow!("stream {id} not found"))?;
                    let distribution = stream
                        .explicit_mut()
                        .ok_or_else(|| anyhow::anyhow!("stream {id} is implicit"))?;
                    distribution.writer = *writer;
                    Ok(TokenAmount::zero())
                }
            }
        }
    }
}

pub(super) fn remove_stream(
    streams: &mut Vec<Stream>,
    tombstones: &mut Vec<Tombstone>,
    accruals: &mut Vec<StreamAccrual>,
    id: StreamId,
) -> Result<TokenAmount> {
    let idx = streams
        .iter()
        .position(|stream| stream.id == id)
        .ok_or_else(|| anyhow::anyhow!("stream {id} not found"))?;
    let mut stream = streams.remove(idx);
    let Some(distribution) = stream.explicit_mut() else {
        return Ok(TokenAmount::zero());
    };
    // Removing without the accrual row would orphan an unknown explicit-stream liability.
    let accrual_idx = accruals
        .iter()
        .position(|row| row.id == id)
        .ok_or_else(|| anyhow::anyhow!("missing accrual for stream {id}"))?;
    let accrual = accruals.remove(accrual_idx);
    let burn = fold(distribution, &accrual.amount)?;
    if !distribution.payable.is_empty() {
        ensure!(
            !tombstones.iter().any(|tombstone| tombstone.id == id),
            "stream ID {id} is already tombstoned"
        );
        tombstones.push(Tombstone { id, payable: std::mem::take(&mut distribution.payable) });
        tombstones.sort_by_key(|tombstone| tombstone.id);
    }
    Ok(burn)
}

pub(super) fn replace_writer(
    streams: &mut [Stream],
    accruals: &mut [StreamAccrual],
    id: StreamId,
    writer: Address,
) -> Result<TokenAmount> {
    let stream = streams
        .iter_mut()
        .find(|stream| stream.id == id)
        .ok_or_else(|| anyhow::anyhow!("stream {id} not found"))?;
    let distribution =
        stream.explicit_mut().ok_or_else(|| anyhow::anyhow!("stream {id} is implicit"))?;
    // Changing writers without the accrual row could reassign an unknown explicit-stream liability.
    let accrual = accruals
        .iter_mut()
        .find(|row| row.id == id)
        .ok_or_else(|| anyhow::anyhow!("missing accrual for stream {id}"))?;
    let burn = fold(distribution, &accrual.amount)?;
    accrual.amount = TokenAmount::zero();
    distribution.writer = writer;
    Ok(burn)
}
