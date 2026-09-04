// Copyright 2019-2022 ChainSafe Systems
// SPDX-License-Identifier: Apache-2.0, MIT

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
    DistributionInit, ExplicitDistribution, settle_period, validate_distribution_init,
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

/// Private tuple stored in `PendingWrite.payload` for `RegisterStream`.
///
/// This is not the public actor-method parameter tuple.
#[derive(Clone, Debug, PartialEq, Eq, Serialize_tuple, Deserialize_tuple)]
pub struct RegisterStreamPayload {
    pub weight: WeightRecord,
    pub distribution: Option<DistributionInit>,
}

/// Private tuple stored in `PendingWrite.payload` for `SetDistribution`.
///
/// This is not the public actor-method parameter tuple.
#[derive(Clone, Debug, PartialEq, Eq, Serialize_tuple, Deserialize_tuple)]
pub struct SetDistributionPayload {
    pub writer: Address,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ApplyResult {
    pub burn: TokenAmount,
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
) -> Result<ChainEpoch> {
    ensure!(
        matches!(op, PendingWriteOp::SetWeightRecords | PendingWriteOp::StepWeightRecords),
        "invalid weight-record operation {op:?}"
    );
    let effective_epoch = timelock_epoch(current_epoch, timelock_epochs)?;
    let mut updates = updates.to_vec();
    updates.sort_by_key(|update| update.id);
    validate_weight_updates(&updates)?;

    let mut proposed = streams.clone();
    ensure_slot_available(&proposed, None, op)?;
    proposed.pending_writes.push(PendingWrite {
        id: None,
        op,
        payload: RawBytes::serialize(&WeightRecordsPayload { updates })?,
        effective_epoch,
    });
    sort_pending(&mut proposed.pending_writes);
    validate_new_pending(streams, &proposed, current_epoch, (None, op))?;

    *streams = proposed;
    Ok(effective_epoch)
}

/// Queues registration at an activation epoch no earlier than the timelock boundary.
pub(crate) fn queue_register_stream(
    streams: &mut StreamsState,
    current_epoch: ChainEpoch,
    timelock_epochs: ChainEpoch,
    stream: Stream,
    activation_epoch: ChainEpoch,
) -> Result<ChainEpoch> {
    ensure!(stream.id != 0, "stream ID 0 is reserved");
    validate_weight_record(&stream.weight)?;
    if let Some(distribution) = &stream.distribution
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
    ensure_slot_available(streams, Some(stream.id), PendingWriteOp::RegisterStream)?;

    let earliest = timelock_epoch(current_epoch, timelock_epochs)?;
    ensure!(
        activation_epoch >= earliest,
        "activation epoch {activation_epoch} is before timelock floor {earliest}"
    );

    let mut proposed = streams.clone();
    proposed.pending_writes.push(PendingWrite {
        id: Some(stream.id),
        op: PendingWriteOp::RegisterStream,
        payload: RawBytes::serialize(&RegisterStreamPayload {
            weight: stream.weight,
            distribution,
        })?,
        effective_epoch: activation_epoch,
    });
    sort_pending(&mut proposed.pending_writes);
    validate_new_pending(
        streams,
        &proposed,
        current_epoch,
        (Some(stream.id), PendingWriteOp::RegisterStream),
    )?;

    *streams = proposed;
    Ok(activation_epoch)
}

/// Queues removal; explicit-stream liabilities are settled when the write applies.
pub(crate) fn queue_remove_stream(
    streams: &mut StreamsState,
    current_epoch: ChainEpoch,
    timelock_epochs: ChainEpoch,
    id: StreamId,
) -> Result<ChainEpoch> {
    let effective_epoch = timelock_epoch(current_epoch, timelock_epochs)?;
    ensure_slot_available(streams, Some(id), PendingWriteOp::RemoveStream)?;

    let mut proposed = streams.clone();
    proposed.pending_writes.push(PendingWrite {
        id: Some(id),
        op: PendingWriteOp::RemoveStream,
        payload: RawBytes::new(EMPTY_TUPLE_CBOR.to_vec()),
        effective_epoch,
    });
    sort_pending(&mut proposed.pending_writes);
    validate_new_pending(
        streams,
        &proposed,
        current_epoch,
        (Some(id), PendingWriteOp::RemoveStream),
    )?;
    validate_tombstone_capacity(&proposed)?;

    *streams = proposed;
    Ok(effective_epoch)
}

/// Queues writer replacement; the outgoing period is settled when the write applies.
pub(crate) fn queue_set_distribution(
    streams: &mut StreamsState,
    current_epoch: ChainEpoch,
    timelock_epochs: ChainEpoch,
    id: StreamId,
    writer: Address,
) -> Result<ChainEpoch> {
    let effective_epoch = timelock_epoch(current_epoch, timelock_epochs)?;
    ensure_slot_available(streams, Some(id), PendingWriteOp::SetDistribution)?;

    let mut proposed = streams.clone();
    proposed.pending_writes.push(PendingWrite {
        id: Some(id),
        op: PendingWriteOp::SetDistribution,
        payload: RawBytes::serialize(&SetDistributionPayload { writer })?,
        effective_epoch,
    });
    sort_pending(&mut proposed.pending_writes);
    validate_new_pending(
        streams,
        &proposed,
        current_epoch,
        (Some(id), PendingWriteOp::SetDistribution),
    )?;

    *streams = proposed;
    Ok(effective_epoch)
}

pub(super) fn cancel_pending(
    streams: &mut StreamsState,
    id: Option<StreamId>,
    op: PendingWriteOp,
) -> Result<Option<PendingWrite>> {
    validate_cancel_target(id, op)?;
    let slot = pending_slot(id, op);
    let removed = streams
        .pending_writes
        .iter()
        .position(|write| pending_slot(write.id, write.op) == slot)
        .map(|idx| streams.pending_writes.remove(idx));
    Ok(removed)
}

pub(crate) fn validate_cancel_target(id: Option<StreamId>, op: PendingWriteOp) -> Result<()> {
    ensure!(op != PendingWriteOp::StepWeightRecords, "StepWeightRecords cannot be cancelled");
    validate_pending_target(id, op)
}

/// Applies writes due through `epoch` before attempting cancellation.
pub(crate) fn apply_due_writes_and_cancel(
    streams: &mut StreamsState,
    accruals: &mut Vec<StreamAccrual>,
    epoch: ChainEpoch,
    id: Option<StreamId>,
    op: PendingWriteOp,
) -> Result<CancelResult> {
    validate_mutation_state(streams, accruals)?;
    let mut projected = project_due_writes(streams, accruals, epoch)?;
    let removed = cancel_pending(&mut projected.streams, id, op)?;
    *streams = projected.streams;
    *accruals = projected.accruals;
    Ok(CancelResult { apply_result: projected.apply_result, removed })
}

/// Applies every write due through `epoch`, each validated from its own effective epoch exactly
/// as admission projected it. From invalid weight state, only writes that restore a valid
/// schedule apply; the rest are dropped atomically.
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
    let mut burn = TokenAmount::zero();
    let mut applied = Vec::new();
    let mut dropped = Vec::new();

    for write in due {
        let mut projected = next_streams.clone();
        let stranded = apply_pending_transition(&mut projected, None, &write)
            .and_then(|_| validate_transition_state(&projected, write.effective_epoch))
            .is_err();
        if stranded {
            dropped.push(write);
            continue;
        }

        let mut candidate_streams = next_streams.clone();
        let mut candidate_accruals = next_accruals.clone();
        let write_burn = apply_pending_transition(
            &mut candidate_streams,
            Some(&mut candidate_accruals),
            &write,
        )?;
        validate_transition_state(&candidate_streams, write.effective_epoch)?;
        next_streams = candidate_streams;
        next_accruals = candidate_accruals;
        applied.push(write);
        burn += write_burn;
    }

    *streams = next_streams;
    *accruals = next_accruals;
    Ok(ApplyResult { burn, applied, dropped })
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

fn pending_slot(id: Option<StreamId>, op: PendingWriteOp) -> (Option<StreamId>, PendingWriteOp) {
    match op {
        PendingWriteOp::SetWeightRecords | PendingWriteOp::StepWeightRecords => (None, op),
        _ => (id, op),
    }
}

fn validate_pending_target(id: Option<StreamId>, op: PendingWriteOp) -> Result<()> {
    let schedule_wide =
        matches!(op, PendingWriteOp::SetWeightRecords | PendingWriteOp::StepWeightRecords);
    match (schedule_wide, id) {
        (true, Some(_)) => Err(anyhow::anyhow!("schedule-wide call has a stream ID")),
        (false, None) => Err(anyhow::anyhow!("per-stream call has no stream ID")),
        _ => Ok(()),
    }
}

fn ensure_slot_available(
    streams: &StreamsState,
    id: Option<StreamId>,
    op: PendingWriteOp,
) -> Result<()> {
    let slot = pending_slot(id, op);
    ensure!(
        !streams.pending_writes.iter().any(|write| pending_slot(write.id, write.op) == slot),
        "pending slot ({:?}, {:?}) is occupied",
        slot.0,
        slot.1
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

fn validate_new_pending(
    current: &StreamsState,
    proposed: &StreamsState,
    current_epoch: ChainEpoch,
    new_slot: (Option<StreamId>, PendingWriteOp),
) -> Result<()> {
    let accepted_before = match validate_projected_queue(current, current_epoch, None) {
        Ok(accepted) => accepted,
        Err(error) if new_slot.1 == PendingWriteOp::SetWeightRecords => {
            validate_projected_queue_recovering(current, current_epoch, None).map_err(|_| error)?
        }
        Err(error) => return Err(error),
    };
    let accepted_after = match validate_projected_queue(proposed, current_epoch, Some(new_slot)) {
        Ok(accepted) => accepted,
        Err(error) if new_slot.1 == PendingWriteOp::SetWeightRecords => {
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
    required_slot: Option<(Option<StreamId>, PendingWriteOp)>,
) -> Result<BTreeSet<(Option<StreamId>, PendingWriteOp)>> {
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
    required_slot: Option<(Option<StreamId>, PendingWriteOp)>,
) -> Result<BTreeSet<(Option<StreamId>, PendingWriteOp)>> {
    validate_projected_queue_inner(streams, current_epoch, required_slot, true, Some(current_epoch))
}

pub(super) fn validate_projected_queue_inner(
    streams: &StreamsState,
    current_epoch: ChainEpoch,
    required_slot: Option<(Option<StreamId>, PendingWriteOp)>,
    allow_invalid_initial_schedule: bool,
    minimum_epoch: Option<ChainEpoch>,
) -> Result<BTreeSet<(Option<StreamId>, PendingWriteOp)>> {
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
        let slot = pending_slot(write.id, write.op);
        // Validated from the entry's effective epoch, as at application, so a null round at
        // that epoch cannot change which writes apply.
        let mut candidate = projected.clone();
        let result = apply_pending_transition(&mut candidate, None, write)
            .and_then(|_| validate_transition_state(&candidate, write.effective_epoch));
        match result {
            Ok(()) => {
                projected = candidate;
                accepted.insert(slot);
            }
            Err(error) if Some(slot) == required_slot => {
                return Err(anyhow::anyhow!(
                    "pending call ({:?}, {:?}) is invalid: {error}",
                    write.id,
                    write.op
                ));
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
        let is_schedule = matches!(
            write.op,
            PendingWriteOp::SetWeightRecords | PendingWriteOp::StepWeightRecords
        );
        ensure!(
            is_schedule == write.id.is_none(),
            "pending call ({:?}, {:?}) has a non-canonical stream ID",
            write.id,
            write.op
        );
        validate_pending_payload(write)?;
        let slot = pending_slot(write.id, write.op);
        ensure!(slots.insert(slot), "duplicate pending slot ({:?}, {:?})", slot.0, slot.1);
        if let Some(epoch) = minimum_epoch
            && write.effective_epoch < epoch
        {
            return Err(anyhow::anyhow!(
                "pending write ({:?}, {:?}) is in the past",
                write.id,
                write.op
            ));
        }
    }
    Ok(())
}

fn validate_pending_payload(write: &PendingWrite) -> Result<()> {
    match write.op {
        PendingWriteOp::SetWeightRecords | PendingWriteOp::StepWeightRecords => {
            let payload: WeightRecordsPayload = write.payload.deserialize()?;
            validate_weight_updates(&payload.updates)?;
        }
        PendingWriteOp::RegisterStream => {
            let payload: RegisterStreamPayload = write.payload.deserialize()?;
            validate_weight_record(&payload.weight)?;
            validate_distribution_init(&payload.distribution)?;
        }
        PendingWriteOp::RemoveStream => {
            ensure!(
                write.payload.bytes() == EMPTY_TUPLE_CBOR,
                "RemoveStream payload is not an empty tuple"
            );
        }
        PendingWriteOp::SetDistribution => {
            let payload: SetDistributionPayload = write.payload.deserialize()?;
            validate_id_address(&payload.writer, "distribution writer")?;
        }
    }
    Ok(())
}

/// Applies one captured call. Supplying accruals enables settlement and liability effects.
fn apply_pending_transition(
    state: &mut StreamsState,
    accruals: Option<&mut Vec<StreamAccrual>>,
    write: &PendingWrite,
) -> Result<TokenAmount> {
    match write.op {
        PendingWriteOp::SetWeightRecords | PendingWriteOp::StepWeightRecords => {
            ensure!(write.id.is_none(), "schedule-wide call has a stream ID");
            let payload: WeightRecordsPayload = write.payload.deserialize()?;
            validate_weight_updates(&payload.updates)?;
            for update in payload.updates {
                let stream = state
                    .streams
                    .iter_mut()
                    .find(|stream| stream.id == update.id)
                    .ok_or_else(|| anyhow::anyhow!("stream {} not found", update.id))?;
                stream.weight = update.weight;
            }
            Ok(TokenAmount::zero())
        }
        PendingWriteOp::RegisterStream => {
            let id =
                write.id.ok_or_else(|| anyhow::anyhow!("RegisterStream call has no stream ID"))?;
            ensure!(id != 0, "stream ID 0 is reserved");
            let payload: RegisterStreamPayload = write.payload.deserialize()?;
            validate_weight_record(&payload.weight)?;
            validate_distribution_init(&payload.distribution)?;
            ensure!(
                !state.streams.iter().any(|stream| stream.id == id),
                "stream ID {id} is already registered"
            );
            ensure!(
                !state.tombstones.iter().any(|tombstone| tombstone.id == id),
                "stream ID {id} is tombstoned"
            );
            let distribution = payload.distribution.map(|distribution| ExplicitDistribution {
                writer: distribution.writer,
                shares: distribution.shares,
                payable: Vec::new(),
                claimed_period: Vec::new(),
            });
            if distribution.is_some()
                && let Some(accruals) = accruals
            {
                accruals.push(StreamAccrual { id, amount: TokenAmount::zero() });
                accruals.sort_by_key(|row| row.id);
            }
            state.streams.push(Stream { id, weight: payload.weight, distribution });
            state.streams.sort_by_key(|stream| stream.id);
            Ok(TokenAmount::zero())
        }
        PendingWriteOp::RemoveStream => {
            let id =
                write.id.ok_or_else(|| anyhow::anyhow!("RemoveStream call has no stream ID"))?;
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
        PendingWriteOp::SetDistribution => {
            let id =
                write.id.ok_or_else(|| anyhow::anyhow!("SetDistribution call has no stream ID"))?;
            let payload: SetDistributionPayload = write.payload.deserialize()?;
            match accruals {
                Some(accruals) => replace_writer(&mut state.streams, accruals, id, payload.writer),
                None => {
                    let stream = state
                        .streams
                        .iter_mut()
                        .find(|stream| stream.id == id)
                        .ok_or_else(|| anyhow::anyhow!("stream {id} not found"))?;
                    let distribution = stream
                        .distribution
                        .as_mut()
                        .ok_or_else(|| anyhow::anyhow!("stream {id} is implicit"))?;
                    distribution.writer = payload.writer;
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
    let Some(distribution) = stream.distribution.as_mut() else {
        return Ok(TokenAmount::zero());
    };
    // Removing without the accrual row would orphan an unknown explicit-stream liability.
    let accrual_idx = accruals
        .iter()
        .position(|row| row.id == id)
        .ok_or_else(|| anyhow::anyhow!("missing accrual for stream {id}"))?;
    let accrual = accruals.remove(accrual_idx);
    let burn = settle_period(distribution, &accrual.amount)?;
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
        stream.distribution.as_mut().ok_or_else(|| anyhow::anyhow!("stream {id} is implicit"))?;
    // Changing writers without the accrual row could reassign an unknown explicit-stream liability.
    let accrual = accruals
        .iter_mut()
        .find(|row| row.id == id)
        .ok_or_else(|| anyhow::anyhow!("missing accrual for stream {id}"))?;
    let burn = settle_period(distribution, &accrual.amount)?;
    accrual.amount = TokenAmount::zero();
    distribution.writer = writer;
    Ok(burn)
}
