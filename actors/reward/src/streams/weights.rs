//! Stream weights: one clamped linear segment per stream, and the envelope over all of them.
//!
//! FIP-0118 2.4.1(3), `ComputeWeight`:
//!
//! ```text
//! WeightRecord = { v_start, slope, t_start, floor, cap }   // one record per stream
//!
//! func clamp(x, lo, hi) -> Fraction:
//!     return max(lo, min(hi, x))
//!
//! func ComputeWeight(w WeightRecord, e Epoch) -> Fraction:
//!     return clamp(w.v_start + w.slope * (e - w.t_start), w.floor, w.cap)
//!
//! // per-epoch weights, all evaluated by the same function:
//! w1(e) = ComputeWeight(W_consensus, e)  // linear: slope < 0, floor = W1_FLOOR, cap = W1_START
//! w2(e) = ComputeWeight(W_service, e)    // Q1: linear bootstrap record mirroring w1's ramp;
//!                                        // Q2 onward: constant record stepped by the gate
//! w0(e) = 1 - sum_{i>=1} ComputeWeight(W_i, e)  // residual over all streams; never stored
//! ```
//!
//! FIP-0118 2.4.8 reduces the envelope to finitely many epochs:
//!
//! ```text
//! The checks: per record, `0 <= floor <= v_start <= cap <= 1`; and the
//! projected weights must satisfy `sum_{i>=1} w_i(e) <= 1` at every epoch,
//! so the burn residual w0 stays non-negative. Each weight is a clamped linear
//! function of the epoch (`ComputeWeight`), so the sum is piecewise
//! linear: between breakpoints it is a straight line, and a line at or
//! below 1 at both ends stays at or below 1 in between. `sum <= 1`
//! therefore need only hold at:
//!
//!   - each record's `t_start`, where its segment begins;
//!
//!   - each epoch where a ramping weight meets a clamp and goes flat:
//!     `e = t_start + (floor - v_start)/slope` and
//!     `e = t_start + (cap - v_start)/slope`, for `slope != 0`;
//!
//!   - one point past the last of these, where every weight has gone
//!     flat and the sum no longer changes.
//! ```
//!
//! - `compute_weight` is `ComputeWeight` using `DENOM` fixed point.
//! - `weight_breakpoints` enumerates that epoch list for one record, bracketing each crossing so
//!   integer division can't step over a one-epoch violation.
//! - `invariants::schedule` sums every stream at every breakpoint from a start epoch onward.
//!
//! The record a stream persists is [`WeightRecord`], in [`crate::state`]; the update and payload
//! shapes an SWA call carries it in are in [`crate::types`].

use std::collections::BTreeSet;

use anyhow::{Result, ensure};
use fvm_shared::clock::ChainEpoch;

use crate::state::{DENOM, WeightRecord};
use crate::types::WeightRecordUpdate;

/// Evaluates a weight at `epoch`, clamped to its inclusive floor and cap.
pub(super) fn compute_weight(record: &WeightRecord, epoch: ChainEpoch) -> u64 {
    let delta = i128::from(epoch) - i128::from(record.t_start);
    // |delta| <= 2^64 - 1 and |slope| <= 2^63, so the product fits i128.
    let product = i128::from(record.slope) * delta;
    // Saturation affects only malformed v_start > DENOM and is equivalent before the u64 clamp.
    let value = i128::from(record.v_start).saturating_add(product);
    u64::try_from(value.min(i128::from(record.cap)).max(i128::from(record.floor)))
        .expect("bounded weight fits u64")
}

pub(super) fn validate_weight_record(record: &WeightRecord) -> Result<()> {
    ensure!(record.floor <= record.cap, "weight floor exceeds cap");
    ensure!(record.v_start >= record.floor, "weight v_start is below floor");
    ensure!(record.v_start <= record.cap, "weight v_start exceeds cap");
    ensure!(record.cap <= DENOM, "weight cap exceeds DENOM");
    Ok(())
}

pub(super) fn validate_weight_updates(updates: &[WeightRecordUpdate]) -> Result<()> {
    ensure!(!updates.is_empty(), "weight-record update is empty");
    for pair in updates.windows(2) {
        ensure!(pair[0].id != pair[1].id, "duplicate weight-record stream ID {}", pair[0].id);
        ensure!(pair[0].id <= pair[1].id, "weight-record updates are not ordered");
    }
    for update in updates {
        validate_weight_record(&update.weight)?;
    }
    Ok(())
}

/// Returns epochs at or after `start_epoch` that can change an admitted record's regime.
/// Adjacent epochs bracket integer crossings so validation cannot skip a one-epoch violation.
pub(super) fn weight_breakpoints(
    record: &WeightRecord,
    start_epoch: ChainEpoch,
) -> Vec<ChainEpoch> {
    // Include the validation boundary and any later record anchor.
    let mut epochs = BTreeSet::from([start_epoch]);
    if record.t_start >= start_epoch {
        epochs.insert(record.t_start);
    }

    if record.slope != 0 {
        // A canonical anchor has one crossing in the slope's forward direction. The opposite
        // crossing can matter only when validation begins before the anchor.
        let start = i128::from(record.t_start);
        let value = i128::from(record.v_start);
        let slope = i128::from(record.slope);
        // The numerator and quotient are within +/-u64::MAX, leaving ample i128 headroom.
        let mut insert_crossing = |bound: u64| {
            let quotient = (i128::from(bound) - value) / slope;
            for offset in [-1_i128, 0, 1] {
                let epoch = start + quotient + offset;
                if let Ok(epoch) = ChainEpoch::try_from(epoch)
                    && epoch >= start_epoch
                {
                    epochs.insert(epoch);
                }
            }
        };

        insert_crossing(if record.slope > 0 { record.cap } else { record.floor });
        if start_epoch < record.t_start {
            insert_crossing(if record.slope > 0 { record.floor } else { record.cap });
        }
    }

    // Sample beyond the last crossing and at the epoch domain's absolute endpoint.
    if let Some(last) = epochs.last().copied()
        && let Some(after) = last.checked_add(1)
    {
        epochs.insert(after);
    }
    epochs.insert(ChainEpoch::MAX);
    epochs.into_iter().collect()
}
