// Copyright 2019-2022 ChainSafe Systems
// SPDX-License-Identifier: Apache-2.0, MIT

use fil_actors_runtime::runtime::Policy;
use fil_actors_runtime::Array;

use fvm_ipld_blockstore::Blockstore;
use fvm_shared::clock::{ChainEpoch, QuantSpec};
use fvm_shared::sector::SectorNumber;

use super::{DeadlineInfo, Deadlines, Partition};

pub fn new_deadline_info(
    policy: &Policy,
    proving_period_start: ChainEpoch,
    deadline_idx: u64,
    current_epoch: ChainEpoch,
) -> DeadlineInfo {
    DeadlineInfo::new(
        proving_period_start,
        deadline_idx,
        current_epoch,
        policy.wpost_period_deadlines,
        policy.wpost_proving_period,
        policy.wpost_challenge_window,
        policy.wpost_challenge_lookback,
        policy.fault_declaration_cutoff,
    )
}

impl Deadlines {
    /// Returns the deadline and partition index for a sector number.
    /// Returns an error if the sector number is not tracked by `self`.
    pub fn find_sector<BS: Blockstore>(
        &self,
        policy: &Policy,
        store: &BS,
        sector_number: SectorNumber,
    ) -> anyhow::Result<(u64, u64)> {
        for i in 0..self.due.len() {
            let deadline_idx = i as u64;
            let deadline = self.load_deadline(policy, store, deadline_idx)?;
            let partitions = Array::<Partition, _>::load(&deadline.partitions, store)?;

            let mut partition_idx = None;

            partitions.for_each_while(|i, partition| {
                if partition.sectors.get(sector_number) {
                    partition_idx = Some(i);
                    Ok(false)
                } else {
                    Ok(true)
                }
            })?;

            if let Some(partition_idx) = partition_idx {
                return Ok((deadline_idx, partition_idx));
            }
        }

        Err(anyhow::anyhow!("sector {} not due at any deadline", sector_number))
    }
}

/// Returns true if the deadline at the given index is currently mutable.
pub fn deadline_is_mutable(
    policy: &Policy,
    proving_period_start: ChainEpoch,
    deadline_idx: u64,
    current_epoch: ChainEpoch,
) -> bool {
    // Get the next non-elapsed deadline (i.e., the next time we care about
    // mutations to the deadline).
    let deadline_info =
        new_deadline_info(policy, proving_period_start, deadline_idx, current_epoch)
            .next_not_elapsed();

    // Ensure that the current epoch is at least one challenge window before
    // that deadline opens.
    current_epoch < deadline_info.open - policy.wpost_challenge_window
}

pub fn quant_spec_for_deadline(policy: &Policy, di: &DeadlineInfo) -> QuantSpec {
    QuantSpec { unit: policy.wpost_proving_period, offset: di.last() }
}

// Returns true if optimistically accepted posts submitted to the given deadline
// may be disputed. Specifically, this ensures that:
//
// 1. Optimistic PoSts may not be disputed while the challenge window is open.
// 2. Optimistic PoSts may not be disputed after the miner could have compacted the deadline.
pub fn deadline_available_for_optimistic_post_dispute(
    policy: &Policy,
    proving_period_start: ChainEpoch,
    deadline_idx: u64,
    current_epoch: ChainEpoch,
) -> bool {
    if proving_period_start > current_epoch {
        return false;
    }
    let dl_info = new_deadline_info(policy, proving_period_start, deadline_idx, current_epoch)
        .next_not_elapsed();

    !dl_info.is_open()
        && current_epoch
            < (dl_info.close - policy.wpost_proving_period) + policy.wpost_dispute_window
}

// Returns true if the given deadline may compacted in the current epoch.
// Deadlines may not be compacted when:
//
// 1. The deadline is currently being challenged.
// 2. The deadline is to be challenged next.
// 3. Optimistically accepted posts from the deadline's last challenge window
//    can currently be disputed.
pub fn deadline_available_for_compaction(
    policy: &Policy,
    proving_period_start: ChainEpoch,
    deadline_idx: u64,
    current_epoch: ChainEpoch,
) -> bool {
    deadline_is_mutable(policy, proving_period_start, deadline_idx, current_epoch)
        && !deadline_available_for_optimistic_post_dispute(
            policy,
            proving_period_start,
            deadline_idx,
            current_epoch,
        )
}

// the distance between from_deadline and to_deadline clockwise in deadline unit.
fn deadline_distance(policy: &Policy, from_deadline: u64, to_deadline: u64) -> u64 {
    if to_deadline >= from_deadline {
        to_deadline - from_deadline
    } else {
        policy.wpost_period_deadlines - from_deadline + to_deadline
    }
}

// only allow moving to a nearer deadline from current one
pub fn deadline_available_for_move(
    policy: &Policy,
    from_deadline: u64,
    to_deadline: u64,
    current_deadline: &DeadlineInfo,
) -> Result<(), String> {
    if !deadline_is_mutable(
        policy,
        current_deadline.period_start,
        from_deadline,
        current_deadline.current_epoch,
    ) {
        return Err(format!(
            "cannot move from a deadline {}, immutable at epoch {}",
            from_deadline, current_deadline.current_epoch
        ));
    }

    if !deadline_is_mutable(
        policy,
        current_deadline.period_start,
        to_deadline,
        current_deadline.current_epoch,
    ) {
        return Err(format!(
            "cannot move to a deadline {}, immutable at epoch {}",
            to_deadline, current_deadline.current_epoch
        ));
    }

    if deadline_distance(policy, current_deadline.index, to_deadline)
        >= deadline_distance(policy, current_deadline.index, from_deadline)
    {
        return Err(format!(
            "can only move to a deadline which is nearer from current deadline {}, to_deadline {} is not nearer than from_deadline {}",
            current_deadline.index, to_deadline, from_deadline
        ));
    }

    Ok(())
}

// returns the nearest deadline info with index `target_deadline` that has already occured from the point of view of the current deadline(including the current deadline).
pub fn nearest_occured_deadline_info(
    policy: &Policy,
    current_deadline: &DeadlineInfo,
    target_deadline: u64,
) -> DeadlineInfo {
    // Find the proving period start for the deadline in question.
    let mut pp_start = current_deadline.period_start;
    if current_deadline.index < target_deadline {
        pp_start -= policy.wpost_proving_period
    }

    new_deadline_info(policy, pp_start, target_deadline, current_deadline.current_epoch)
}

// Determine current period start and deadline index directly from current epoch and
// the offset implied by the proving period. This works correctly even for the state
// of a miner actor without an active deadline cron
pub fn new_deadline_info_from_offset_and_epoch(
    policy: &Policy,
    period_start_seed: ChainEpoch,
    current_epoch: ChainEpoch,
) -> DeadlineInfo {
    let q = QuantSpec { unit: policy.wpost_proving_period, offset: period_start_seed };
    let current_period_start = q.quantize_down(current_epoch);
    let current_deadline_idx =
        ((current_epoch - current_period_start) / policy.wpost_challenge_window) as u64;
    new_deadline_info(policy, current_period_start, current_deadline_idx, current_epoch)
}
