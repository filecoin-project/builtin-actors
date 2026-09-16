//! Stream weights: one clamped linear segment per stream, and the envelope over all of them.
//!
//! A [`WeightRecord`] is a line clamped to a band, `clamp(v_start + slope * (e - t_start),
//! floor, cap)`, in `DENOM` fixed point. The residual is whatever the stored weights leave of
//! `DENOM`, this is burnt.
//!
//! The envelope is the rule that the stored weights sum to (strictly) _at most_ `DENOM` at every
//! epoch projected out into the future, so the residual never goes negative. Each weight is
//! piecewise linear, so the sum is too, therefore a line under `DENOM` at both ends stays under it
//! between them. So we can run the check at a finite number of epochs to be sure it holds
//! everywhere:
//!
//! 1. each record's `t_start`, where its segment begins;
//! 2. each epoch where a ramp meets its floor or cap and goes flat;
//! 3. one epoch past the last of those, after which the sum is constant.
//!
//! - `compute_weight` is the clamped line.
//! - `weight_breakpoints` lists those epochs for one record, bracketing each crossing so
//!   integer division cannot step over a one-epoch violation.
//! - `invariants::schedule` sums every stream at every breakpoint from a start epoch onward.
//!
//! The record a stream persists is [`WeightRecord`], in [`crate::state`]; the update and payload
//! encoded forms from SWA calls are in [`crate::types`].

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
    ensure!(!updates.is_empty(), "weight-record batch has no updates");
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
