//! State validation, in three tiers.
//!
//! FIP-0118 2.4.3 and 2.4.8 distinguish three tiers of state trust:
//!
//! - **structure**: sorted unique keys, bounds, deferred payloads decoding to their canonical
//!   shape, stream IDs disjoint across live streams, tombstones and pending registrations,
//!   tombstone capacity.
//! - **accounting**: accrual rows corresponding one to one with the live explicit streams,
//!   non-negative accruals, positive recipient rows, each claimed amount within what its
//!   recipient has earned.
//! - **schedule**: every weight record in band, and the aggregate within `DENOM` at every
//!   breakpoint from a given epoch onward.
//!
//! Each entry point requires a different set, and fails differently when one does not hold:
//!
//! | entry | structure | accounting | schedule |
//! |---|---|---|---|
//! | explicit method, at load | abort | abort | not required (2.4.8) |
//! | admission of a queued call | held | held | required on the projection; `SetWeightRecords` may repair |
//! | award | gas only | gas only | gas only |
//! | invariant checker | reported | reported | reported |
//!
//! One function per tier, and each runs once. [`structure`] and [`accounting`] run together when
//! a [`Ledger`](super::Ledger) is constructed, which is the only way an operation receives state,
//! so operations below that point assume both and re-prove neither. [`schedule`] is not held at
//! load, because claims, cancellation and `SetShares` stay usable from an invalid schedule
//! (2.4.8); the queue applies it from the epoch each write becomes effective, and the award
//! applies it at its own epoch. [`validate_streams_state`] runs all three plus the projected
//! queue, and is what `testing.rs` reports from.

use std::collections::BTreeSet;

use anyhow::{Result, ensure};
use fvm_shared::clock::ChainEpoch;

use super::distribution::{
    validate_amount_rows, validate_id_address, validate_period_claims, validate_stored_shares,
};
use super::queue::{PendingWriteOp, validate_pending_queue, validate_projected_queue_inner};
use super::weights::validate_weight_schedule;
use super::{
    MAX_PAYABLE_ROWS_PER_STREAM, MAX_RECIPIENTS, MAX_STREAMS, MAX_TOMBSTONE_ROWS, Stream,
    StreamAccrual, StreamsState,
};

/// The structure invariants tier: the shape of the streams block, independent of accounting and
/// weights.
pub(super) fn structure(streams: &StreamsState) -> Result<()> {
    validate_pending_queue(&streams.pending_writes)?;
    stream_table(&streams.streams)?;
    ensure!(streams.tombstones.is_sorted_by(|a, b| a.id < b.id), "tombstones are not ordered");

    let live_ids: BTreeSet<_> = streams.streams.iter().map(|stream| stream.id).collect();
    let tombstone_ids: BTreeSet<_> =
        streams.tombstones.iter().map(|tombstone| tombstone.id).collect();
    ensure!(live_ids.is_disjoint(&tombstone_ids), "a stream ID is live and tombstoned");
    for tombstone in &streams.tombstones {
        ensure!(tombstone.id != 0, "stream ID 0 is reserved");
        ensure!(!tombstone.payable.is_empty(), "tombstone {} is empty", tombstone.id);
        validate_amount_rows(&tombstone.payable, "tombstone payable")?;
    }
    for write in &streams.pending_writes {
        if write.op == PendingWriteOp::RegisterStream {
            // Pending-queue shape validation requires IDs on every per-stream operation.
            let id = write.id.expect("validated per-stream pending call has an ID");
            ensure!(
                !live_ids.contains(&id) && !tombstone_ids.contains(&id),
                "pending registration reuses stream ID {id}"
            );
        }
    }
    validate_tombstone_capacity(streams)?;
    Ok(())
}

/// The accounting invariants tier: the accrual rows against the explicit streams they belong to.
pub(super) fn accounting(streams: &StreamsState, accrued: &[StreamAccrual]) -> Result<()> {
    ensure!(accrued.is_sorted_by(|a, b| a.id < b.id), "explicit-stream accruals are not ordered");

    let explicit_ids: BTreeSet<_> = streams
        .streams
        .iter()
        .filter(|stream| !stream.is_implicit())
        .map(|stream| stream.id)
        .collect();
    let accrual_ids: BTreeSet<_> = accrued.iter().map(|row| row.id).collect();
    ensure!(
        explicit_ids == accrual_ids,
        "explicit-stream accrual IDs do not match live explicit streams"
    );
    for accrual in accrued {
        ensure!(
            !accrual.amount.is_negative(),
            "explicit-stream accrual {} is negative",
            accrual.id
        );
        // Exact accrual-ID equality above proves this is a live explicit stream.
        let distribution = streams
            .streams
            .iter()
            .find(|stream| stream.id == accrual.id)
            .and_then(Stream::explicit)
            .expect("explicit-stream accrual IDs matched explicit streams");
        validate_period_claims(distribution, &accrual.amount)?;
    }
    Ok(())
}

/// The schedule invariants tier: every record in band and the envelope within `DENOM` from `from`
/// onward.
pub(super) fn schedule(streams: &[Stream], from: ChainEpoch) -> Result<()> {
    validate_weight_schedule(streams, from)
}

/// The live stream table and its stored rows: the part of [`structure`] a transition can break.
pub(super) fn stream_table(streams: &[Stream]) -> Result<()> {
    ensure!(streams.len() <= MAX_STREAMS, "stream count exceeds maximum {MAX_STREAMS}");
    ensure!(streams.is_sorted_by(|a, b| a.id < b.id), "stream IDs are not ordered");
    ensure!(!streams.iter().any(|stream| stream.id == 0), "stream ID 0 is reserved");
    ensure!(
        streams.iter().filter(|stream| stream.is_implicit()).count() <= 1,
        "multiple implicit streams"
    );
    for stream in streams {
        if let Some(distribution) = stream.explicit() {
            validate_id_address(&distribution.writer, "distribution writer")?;
            validate_stored_shares(&distribution.shares)?;
            validate_amount_rows(&distribution.payable, "payable")?;
            validate_amount_rows(&distribution.claimed_period, "claimed-period")?;
            let reserved_rows = distribution.payable.union_len(&distribution.shares);
            ensure!(
                reserved_rows <= MAX_PAYABLE_ROWS_PER_STREAM,
                "stream {} payable row reservation {reserved_rows} exceeds maximum {MAX_PAYABLE_ROWS_PER_STREAM}",
                stream.id
            );
            ensure!(
                distribution.claimed_period.len() <= MAX_RECIPIENTS,
                "stream {} claimed-period row count {} exceeds maximum {MAX_RECIPIENTS}",
                stream.id,
                distribution.claimed_period.len()
            );
        }
    }
    Ok(())
}

/// Validates persisted stream state and its queued schedule at `current_epoch`.
pub(crate) fn validate_streams_state(
    streams: &StreamsState,
    accrued: &[StreamAccrual],
    current_epoch: ChainEpoch,
) -> Result<()> {
    structure(streams)?;
    accounting(streams, accrued)?;
    schedule(&streams.streams, current_epoch)?;
    // Persisted writes may be past due after null rounds or a quiet stream-engine interval.
    validate_projected_queue_inner(streams, current_epoch, None, false, None)?;
    Ok(())
}

/// The tombstone row bound, which reserves `MAX_RECIPIENTS` or the removal's own union for every
/// pending removal.
pub(super) fn validate_tombstone_capacity(streams: &StreamsState) -> Result<()> {
    let mut rows: usize = streams.tombstones.iter().map(|tombstone| tombstone.payable.len()).sum();
    for write in &streams.pending_writes {
        if write.op != PendingWriteOp::RemoveStream {
            continue;
        }
        let id = write.id.expect("validated removal has a stream ID");
        rows += streams
            .streams
            .iter()
            .find(|stream| stream.id == id)
            .and_then(Stream::explicit)
            .map_or(MAX_RECIPIENTS, |distribution| {
                MAX_RECIPIENTS.max(distribution.payable.union_len(&distribution.shares))
            });
    }
    ensure!(
        rows <= MAX_TOMBSTONE_ROWS,
        "tombstone row reservation {rows} exceeds maximum {MAX_TOMBSTONE_ROWS}"
    );
    Ok(())
}
