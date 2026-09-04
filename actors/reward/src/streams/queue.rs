//! The pending-write queue: the SWA timelock, admission, cancellation, and application.
//!
//! Every SWA write lands here first as a `PendingWrite` containing the op, its encoded params,
//! and the epoch it becomes due. Each one sits in a [`Slot`]. Per-stream ops key by `(id, op)`;
//! the two weight ops are schedule-wide and key by `op` alone (i.e. null id). One write per slot,
//! and an occupied slot means a new write for that slot will be rejected, so changing your mind
//! means a cancel plus requeue and the timelock starts again. `CancelPending` has to name the slot
//! being cancelled the same way the queue keys it. For cancellation a mismatched id/op is an error,
//! an empty slot is a no-op, and `StepWeightRecords` can't be cancelled at all (the discretionary
//! `SetWeightRecords` can be).
//!
//! Three epochs matter for an entry. The queue epoch is when the SWA called. The effective epoch
//! is queue plus `swa_timelock_epochs` (except for `RegisterStream`, which brings its own
//! `activation_epoch` and just has to be at or past that floor). The apply epoch is whenever the
//! next stream-engine method runs at or after that, `AwardBlockReward` included, because every
//! one of them applies due entries before doing its own thing. The queue is sorted by effective
//! epoch with ties in arrival order, so the head tells you whether anything is due.
//!
//! One rule drives most of the logic in here: *cancellation is unconditional*. A compromised SWA
//! must not be able to make a bad write uncancellable by queueing something that depends on it,
//! and both governance multisigs may relay the same objection. So an admitted write can lose a
//! prerequisite after admission, application has to be able to drop it, and admission has to
//! validate against the state _as it will be at application_ rather than today's, since two
//! writes, each fine on their own, can sum past one together. That's why both
//! [`Ledger::admit`] and [`Ledger::apply_due`] replay the queue in effective order.
//!
//! The admission rule: *W is admitted iff, applying the queue in effective order, W applies in
//! the state its predecessors produce, and every write that applied before W's admission still
//! applies after it*. The second clause is the subset check in [`Ledger::admit`]. It's
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
//! At application, due entries go in queue order, each validated from its own effective epoch
//! rather than the current one, so a null round can't change which ones apply. A well-formed
//! entry that's lost its prerequisite is dropped with a `write-dropped` event and we carry on;
//! the installed config is untouched.
//!
//! If the persisted weight records or envelope are already invalid, due entries still only
//! apply when their result validates, and new ones are rejected, with one exception: a
//! `SetWeightRecords` that repairs things without stranding a still-valid entry. It gets the
//! ordinary timelock like anything else.
//!
//! FIP-0118 2.4.7 has the timelock, the three epochs, slots and cancellability; 2.4.8 has details
//! about what admission proves, stranding, and repair.

use std::collections::BTreeSet;
use std::fmt;

use anyhow::{Result, ensure};
use fvm_ipld_encoding::RawBytes;
use fvm_shared::address::Address;
use fvm_shared::clock::ChainEpoch;
use fvm_shared::econ::TokenAmount;
use log::info;
use num_traits::Zero;

use super::Ledger;
use super::distribution::{validate_distribution_init, validate_id_address};
use super::invariants::{schedule, validate_tombstone_capacity};
use super::weights::{validate_weight_record, validate_weight_updates};
use crate::state::{
    ExplicitDistribution, MAX_PENDING_WRITES, MAX_STREAMS, PendingWrite, PendingWriteOp,
    RecipientTable, Stream, StreamId, StreamsState, WeightRecord,
};
use crate::types::{
    DistributionInit, RegisterStreamPayload, SetDistributionPayload, WeightRecordUpdate,
    WeightRecordsPayload,
};

const EMPTY_TUPLE_CBOR: &[u8] = &[0x80];

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

impl PendingWrite {
    /// The slot this write occupies.
    fn slot(&self) -> Slot {
        Slot::for_target(self.id, self.op)
            .expect("structure invariants: every queued write names a slot")
    }
}

/// A prerequisite a queued write can lose between admission and application.
///
/// Only cancellation takes one away (see the module doc).
#[derive(Debug)]
pub(super) enum Stranded {
    /// A weights update, removal or writer change names an ID that is not live.
    MissingStream(StreamId),
    /// A writer change names the implicit stream.
    NotExplicit(StreamId),
    /// A registration's ID is live or tombstoned.
    StreamIdInUse(StreamId),
    /// A registration has no room left in the stream table.
    StreamTableFull,
    /// A registration would give the schedule a second implicit stream.
    SecondImplicit,
    /// A registration names the reserved ID 0.
    ReservedId,
    /// The weight schedule does not hold from the write's effective epoch onward, either because
    /// a record is out of band or because the envelope exceeds `DENOM` at some epoch.
    Schedule(anyhow::Error),
}

impl fmt::Display for Stranded {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Stranded::MissingStream(id) => write!(f, "stream {id} not found"),
            Stranded::NotExplicit(id) => write!(f, "stream {id} is implicit"),
            Stranded::StreamIdInUse(id) => write!(f, "stream ID {id} is live or tombstoned"),
            Stranded::StreamTableFull => {
                write!(f, "stream count exceeds maximum {MAX_STREAMS}")
            }
            Stranded::SecondImplicit => write!(f, "multiple implicit streams"),
            Stranded::ReservedId => write!(f, "stream ID 0 is reserved"),
            Stranded::Schedule(error) => write!(f, "{error}"),
        }
    }
}

/// Whether the weight schedule holds before the queue is replayed.
///
/// `Repairing` is the exception FIP-0118 2.4.8 allows: from an invalid schedule a
/// `SetWeightRecords` batch may still be admitted, provided it puts the schedule back.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Baseline {
    Valid,
    Repairing,
}

/// The queue replayed in effective order: the slots that apply, and the prerequisite each of the
/// others is missing.
#[derive(Debug, Default)]
struct Projection {
    accepted: BTreeSet<Slot>,
    stranded: Vec<(Slot, Stranded)>,
}

/// A queued write's operation and arguments, decoded from its payload.
///
/// Every read of a payload produces one of these and every write of a payload comes from one, so
/// the CBOR in `PendingWrite.payload` is confined to [`QueuedCall::decode`] and
/// [`QueuedCall::encode`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum QueuedCall {
    Weights {
        op: PendingWriteOp,
        updates: Vec<WeightRecordUpdate>,
    },
    Register {
        id: StreamId,
        weight: WeightRecord,
        distribution: Option<DistributionInit>,
        /// The epoch the stream activates, which is the entry's effective epoch rather than a
        /// payload field.
        activation: ChainEpoch,
    },
    Remove {
        id: StreamId,
    },
    SetDistribution {
        id: StreamId,
        writer: Address,
    },
}

impl QueuedCall {
    /// Puts a call's weight updates in stream ID order and its share rows in recipient order.
    ///
    /// A caller may send those rows in any order, but the queue stores them sorted and every
    /// validator downstream requires that, so the actor sorts each call as it builds it.
    pub(crate) fn canonical(mut self) -> QueuedCall {
        match &mut self {
            QueuedCall::Weights { updates, .. } => updates.sort_by_key(|update| update.id),
            QueuedCall::Register { distribution: Some(distribution), .. } => {
                distribution.shares.sort_by_key(|row| row.recipient)
            }
            QueuedCall::Register { .. }
            | QueuedCall::Remove { .. }
            | QueuedCall::SetDistribution { .. } => {}
        }
        self
    }

    /// Reads a queue entry's payload, rejecting one that disagrees with the entry's ID and
    /// operation or does not hold that operation's canonical tuple.
    fn decode(write: &PendingWrite) -> Result<QueuedCall> {
        let call = match Slot::for_target(write.id, write.op)? {
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
                    activation: write.effective_epoch,
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
        };
        // The validators above accept sorted rows only, so a stored payload is already sorted.
        debug_assert_eq!(
            call.clone().canonical(),
            call,
            "structure invariants: every queued payload is sorted"
        );
        Ok(call)
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

/// The effects of applying due writes, for the actor layer to settle after the transaction.
///
/// This crosses Rust call boundaries only so doesn't need to be encodable.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct ApplyResult {
    pub fold_dust: TokenAmount,
    /// Successful writes, for actor-layer events after a committed application.
    pub applied: Vec<PendingWrite>,
    /// Removed writes, for actor-layer events after a committed application.
    pub dropped: Vec<PendingWrite>,
}

impl Ledger {
    /// Applies one queued call in the state its predecessors left, from the epoch it becomes
    /// effective.
    ///
    /// Returns the fold dust for the caller to burn, or the one prerequisite the call is missing.
    /// Only `RemoveStream` and `SetDistribution` fold, closing the explicit stream's period before
    /// it is tombstoned or re-pointed, so only they can leave dust; the other calls return zero.
    fn apply(&mut self, call: &QueuedCall, effective: ChainEpoch) -> Result<TokenAmount, Stranded> {
        let dust = match call {
            QueuedCall::Weights { updates, .. } => {
                for update in updates {
                    let stream = self
                        .streams
                        .stream_mut(update.id)
                        .ok_or(Stranded::MissingStream(update.id))?;
                    stream.weight = update.weight.clone();
                }
                TokenAmount::zero()
            }
            QueuedCall::Register { id, weight, distribution, .. } => {
                let id = *id;
                if id == 0 {
                    return Err(Stranded::ReservedId);
                }
                if self.streams.has_stream(id) || self.streams.has_tombstone(id) {
                    return Err(Stranded::StreamIdInUse(id));
                }
                if self.streams.streams.len() >= MAX_STREAMS {
                    return Err(Stranded::StreamTableFull);
                }
                if distribution.is_none() && self.streams.streams.iter().any(Stream::is_implicit) {
                    return Err(Stranded::SecondImplicit);
                }
                let distribution = distribution.as_ref().map(|distribution| ExplicitDistribution {
                    writer: distribution.writer,
                    shares: distribution.shares.clone(),
                    payable: RecipientTable::default(),
                    claimed_period: RecipientTable::default(),
                });
                if distribution.is_some() {
                    self.insert_accrual(id);
                }
                self.streams.insert_stream(Stream { id, weight: weight.clone(), distribution });
                TokenAmount::zero()
            }
            QueuedCall::Remove { id } => self.remove_stream(*id)?,
            QueuedCall::SetDistribution { id, writer } => self.replace_writer(*id, *writer)?,
        };
        // Only the weight envelope needs checking here. The stream table is guarded by the
        // registration preconditions above, a fold only moves value between existing recipients,
        // inserts stay sorted and positive, and tombstone room was charged when the removal was
        // admitted.
        schedule(&self.streams.streams, effective).map_err(Stranded::Schedule)?;
        Ok(dust)
    }

    /// Applies every write due through `epoch`, each validated from its own effective epoch just
    /// as its admission projected. From invalid weight state, only writes that restore a valid
    /// schedule apply and the rest are dropped.
    pub(crate) fn apply_due(&mut self, epoch: ChainEpoch) -> ApplyResult {
        let due_count = self
            .streams
            .pending_writes
            .iter()
            .take_while(|write| write.effective_epoch <= epoch)
            .count();
        let due: Vec<PendingWrite> = self.streams.pending_writes.drain(..due_count).collect();
        let mut result = ApplyResult::default();

        for write in due {
            let call = QueuedCall::decode(&write)
                .expect("structure invariants: every queued payload decodes");
            // Each call runs on a copy of the ledger. If it applies, the copy becomes the ledger;
            // if it's dropped, the copy is thrown away and the ledger is as the last write left it.
            let mut next = self.clone();
            match next.apply(&call, write.effective_epoch) {
                Ok(dust) => {
                    *self = next;
                    result.fold_dust += dust;
                    result.applied.push(write);
                }
                Err(stranded) => {
                    info!(
                        "dropping pending write {:?} effective at {}: {stranded}",
                        call.slot(),
                        write.effective_epoch
                    );
                    result.dropped.push(write);
                }
            }
        }

        self.streams_dirty |= !(result.applied.is_empty() && result.dropped.is_empty());
        result
    }

    /// Checks a call against the current state and returns the epoch it would become effective.
    ///
    /// This looks only at the call and the state as it is now. Checking the call against the
    /// rest of the queue happens afterwards, in [`Ledger::admit`].
    fn admission_preconditions(
        &self,
        call: &QueuedCall,
        epoch: ChainEpoch,
        timelock: ChainEpoch,
    ) -> Result<ChainEpoch> {
        Ok(match call {
            QueuedCall::Weights { op, updates } => {
                ensure!(op.is_schedule_wide(), "invalid weight-record operation {op:?}");
                let effective = timelock_epoch(epoch, timelock)?;
                validate_weight_updates(updates)?;
                effective
            }
            QueuedCall::Register { id, weight, distribution, activation } => {
                ensure!(*id != 0, "stream ID 0 is reserved");
                validate_weight_record(weight)?;
                validate_distribution_init(distribution)?;
                ensure_stream_id_available(&self.streams, *id)?;
                let earliest = timelock_epoch(epoch, timelock)?;
                ensure!(
                    *activation >= earliest,
                    "activation epoch {activation} is before timelock floor {earliest}"
                );
                *activation
            }
            QueuedCall::Remove { .. } => timelock_epoch(epoch, timelock)?,
            QueuedCall::SetDistribution { writer, .. } => {
                validate_id_address(writer, "distribution writer")?;
                timelock_epoch(epoch, timelock)?
            }
        })
    }

    /// Queues one SWA call, rejecting it unless it applies in the state its predecessors produce
    /// and leaves every already-admitted call still applying.
    ///
    /// Returns the entry it inserted, for the actor layer's `write-queued` event.
    pub(crate) fn admit(
        &mut self,
        call: QueuedCall,
        epoch: ChainEpoch,
        timelock: ChainEpoch,
    ) -> Result<&PendingWrite> {
        let effective = self.admission_preconditions(&call, epoch, timelock)?;
        let slot = call.slot();
        ensure_slot_available(&self.streams, slot)?;
        // Admission is the only thing that lengthens the queue, so this is the point where we need
        // to do the bounds check.
        let count = self.streams.pending_writes.len() + 1;
        ensure!(
            count <= MAX_PENDING_WRITES,
            "pending write count {count} exceeds maximum {MAX_PENDING_WRITES}"
        );
        // Every method applies due writes before its own work, so nothing here is in the past.
        debug_assert!(
            self.streams.pending_writes.iter().all(|write| write.effective_epoch >= epoch),
            "admission projects a queue that is entirely ahead of it"
        );

        // The queue as it stands has to project, which is also where the baseline comes from: from
        // an invalid schedule, a SetWeightRecords batch is the one admissible repair (2.4.8).
        let (baseline, before) = match self.project(epoch, Baseline::Valid) {
            Ok(before) => (Baseline::Valid, before),
            Err(_) if slot == Slot::ScheduleWide(PendingWriteOp::SetWeightRecords) => {
                (Baseline::Repairing, self.project(epoch, Baseline::Repairing)?)
            }
            Err(error) => return Err(error),
        };

        self.streams.pending_writes.push(call.queue_entry(effective)?);
        // Stable sorting preserves insertion order among calls effective at the same epoch.
        self.streams.pending_writes.sort_by_key(|write| write.effective_epoch);
        self.streams_dirty = true;

        let after = self.project(epoch, baseline)?;
        if let Some((_, stranded)) = after.stranded.iter().find(|(occupied, _)| *occupied == slot) {
            return Err(anyhow::anyhow!("pending call {slot:?} is invalid: {stranded}"));
        }
        ensure!(
            before.accepted.is_subset(&after.accepted),
            "new call invalidates an existing pending call"
        );
        if let QueuedCall::Remove { .. } = call {
            // A removal reserves the tombstone rows its fold may leave behind.
            validate_tombstone_capacity(&self.streams)?;
        }

        Ok(self
            .streams
            .pending_writes
            .iter()
            .find(|write| write.slot() == slot)
            .expect("the admitted call occupies its slot"))
    }

    /// Empties one queue slot. Cancelling an empty slot is a no-op.
    pub(crate) fn cancel(&mut self, slot: Slot) -> Option<PendingWrite> {
        // Cancellation rewrites the streams block whether or not the slot held a write.
        self.streams_dirty = true;
        let idx = self.streams.pending_writes.iter().position(|write| write.slot() == slot)?;
        Some(self.streams.pending_writes.remove(idx))
    }

    /// Replays the queue in effective order on a copy of this ledger, through the same transition
    /// application uses. Calls stranded by cancellation become future drops.
    fn project(&self, epoch: ChainEpoch, baseline: Baseline) -> Result<Projection> {
        if baseline == Baseline::Valid {
            schedule(&self.streams.streams, epoch)?;
        }
        let mut projected = self.clone();
        let mut projection = Projection::default();

        for write in &self.streams.pending_writes {
            let call = QueuedCall::decode(write)
                .expect("structure invariants: every queued payload decodes");
            // Validated from the entry's effective epoch, as at application, so a null round at
            // that epoch cannot change which writes apply.
            let mut candidate = projected.clone();
            match candidate.apply(&call, write.effective_epoch) {
                Ok(_) => {
                    projected = candidate;
                    projection.accepted.insert(call.slot());
                }
                Err(stranded) => projection.stranded.push((call.slot(), stranded)),
            }
        }
        Ok(projection)
    }
}

fn timelock_epoch(current_epoch: ChainEpoch, timelock_epochs: ChainEpoch) -> Result<ChainEpoch> {
    ensure!(timelock_epochs >= 0, "timelock is negative");
    current_epoch
        .checked_add(timelock_epochs)
        .ok_or_else(|| anyhow::anyhow!("timelock epoch overflow"))
}

fn ensure_slot_available(streams: &StreamsState, slot: Slot) -> Result<()> {
    ensure!(
        !streams.pending_writes.iter().any(|write| write.slot() == slot),
        "pending slot {slot:?} is occupied"
    );
    Ok(())
}

fn ensure_stream_id_available(streams: &StreamsState, id: StreamId) -> Result<()> {
    ensure!(!streams.has_stream(id), "stream ID {id} is already registered");
    ensure!(!streams.has_tombstone(id), "stream ID {id} is tombstoned");
    ensure!(
        !streams
            .pending_writes
            .iter()
            .any(|write| write.id == Some(id) && write.op == PendingWriteOp::RegisterStream),
        "stream ID {id} has a pending registration"
    );
    Ok(())
}

/// Check the queue according to the structure invariants: ordered, bounded, one write per slot, and
/// every payload decodable. Everything downstream reads a slot or a payload without re-proving it.
pub(super) fn validate_pending_queue(writes: &[PendingWrite]) -> Result<()> {
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
    }
    Ok(())
}
