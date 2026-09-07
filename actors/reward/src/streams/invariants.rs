//! What has to be true of the persisted state in two distinct groups, and who checks each.
//!
//! - **structure**: keys sorted and unique, sizes within the bounds, deferred payloads that
//!   decode to their canonical shape, and stream IDs disjoint across live streams and pending
//!   registrations.
//! - **schedule**: every weight record in band, and the weights summing to at most `DENOM` at
//!   every breakpoint from a given epoch onward.
//!
//! Who needs which, and what happens when one fails:
//!
//! | entry | structure | schedule |
//! |---|---|---|
//! | explicit method, at load | abort | not required (2.4.8) |
//! | admission of a queued call | held | required on the projection; `SetWeightRecords` may repair |
//! | award | gas only | gas only |
//! | invariant checker | reported | reported |
//!
//! Each group is checked once, where it's needed. [`structure`] runs when a
//! [`Ledger`](super::Ledger) is built, and building one is the only way an operation gets its
//! state, so nothing past that point checks it again. [`schedule`] is deliberately not part
//! of that: cancellation and `SetShares` have to keep working while the schedule is broken
//! (FIP-0118 2.4.8), so the queue checks it from each write's effective epoch and [`schedule_at`]
//! checks it at the award's epoch. [`validate_streams_state`] runs both and is what `testing.rs`
//! reports from.

use std::collections::BTreeSet;

use anyhow::{Result, ensure};
use fvm_shared::clock::ChainEpoch;

use super::distribution::{validate_id_address, validate_stored_shares};
use super::queue::validate_pending_queue;
use super::weights::{compute_weight, validate_weight_record, weight_breakpoints};
use crate::state::{DENOM, MAX_STREAMS, PendingWriteOp, Stream, StreamsState};

/// The structure invariants: the shape of the streams block, independent of the weights.
pub(crate) fn structure(streams: &StreamsState) -> Result<()> {
    validate_pending_queue(&streams.pending_writes)?;
    stream_table(&streams.streams)?;

    let live_ids: BTreeSet<_> = streams.streams.iter().map(|stream| stream.id).collect();
    for write in &streams.pending_writes {
        if write.op == PendingWriteOp::RegisterStream {
            // Pending-queue shape validation requires IDs on every per-stream operation.
            let id = write.id.expect("validated per-stream pending call has an ID");
            ensure!(!live_ids.contains(&id), "pending registration reuses stream ID {id}");
        }
    }
    Ok(())
}

/// The schedule invariants: every record in band and the envelope within `DENOM` from `from`
/// onward.
///
/// The sum is piecewise linear, so checking it at every record's breakpoints and at the epoch
/// domain's endpoint covers every epoch in between.
pub(super) fn schedule(streams: &[Stream], from: ChainEpoch) -> Result<()> {
    let mut epochs = BTreeSet::from([from, ChainEpoch::MAX]);
    for stream in streams {
        validate_weight_record(&stream.weight)?;
        epochs.extend(weight_breakpoints(&stream.weight, from));
    }

    for epoch in epochs {
        envelope_at(streams, epoch)?;
    }
    Ok(())
}

/// The schedule invariants at a single epoch alone, returning the weights it evaluated, one per
/// stream in stream order.
///
/// The award pays this block and nothing later, so a schedule that holds now and breaks at some
/// future epoch still pays now. [`schedule`] is the stronger property that admission and
/// application require, since a queued write has to hold from its effective epoch onward.
pub(super) fn schedule_at(streams: &[Stream], epoch: ChainEpoch) -> Result<Vec<u64>> {
    for stream in streams {
        validate_weight_record(&stream.weight)?;
    }
    envelope_at(streams, epoch)
}

/// The weights at one epoch, summing within `DENOM` so the burn residual stays non-negative.
fn envelope_at(streams: &[Stream], epoch: ChainEpoch) -> Result<Vec<u64>> {
    let evaluated: Vec<u64> =
        streams.iter().map(|stream| compute_weight(&stream.weight, epoch)).collect();
    let sum: u128 = evaluated.iter().copied().map(u128::from).sum();
    ensure!(sum <= u128::from(DENOM), "stream weights exceed DENOM at epoch {epoch}: {sum}");
    Ok(evaluated)
}

/// The live stream table and its stored rows.
fn stream_table(streams: &[Stream]) -> Result<()> {
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
        }
    }
    Ok(())
}

/// Validates persisted stream state and its schedule at `current_epoch`.
pub(crate) fn validate_streams_state(
    streams: &StreamsState,
    current_epoch: ChainEpoch,
) -> Result<()> {
    structure(streams)?;
    schedule(&streams.streams, current_epoch)
}
