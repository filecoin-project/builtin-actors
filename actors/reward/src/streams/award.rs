// Copyright 2019-2022 ChainSafe Systems
// SPDX-License-Identifier: Apache-2.0, MIT

use std::collections::BTreeSet;

use anyhow::{Result, ensure};
use fvm_shared::bigint::BigInt;
use fvm_shared::clock::ChainEpoch;
use fvm_shared::econ::TokenAmount;
use num_traits::Zero;

use super::distribution::{stored_share_total, validate_amount_rows, validate_period_claims};
use super::weights::{compute_weight, validate_weight_record};
use super::{DENOM, Stream, StreamAccrual, StreamsState};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RewardAllocation {
    pub miner: TokenAmount,
    pub service: Vec<StreamAccrual>,
    pub burn: TokenAmount,
    /// False when weight state is invalid; explicit portions are then skipped.
    pub schedule_valid: bool,
}

/// Splits one block reward at `epoch`; invalid weight state allocates no portion.
pub(crate) fn allocate_reward(
    streams: &[Stream],
    epoch: ChainEpoch,
    block_reward: &TokenAmount,
) -> Result<RewardAllocation> {
    ensure!(!block_reward.is_negative(), "block reward is negative");

    let mut miner = TokenAmount::zero();
    let mut service = Vec::with_capacity(streams.len());
    let mut burn = TokenAmount::zero();
    let mut allocated = TokenAmount::zero();
    let denom = BigInt::from(DENOM);
    let mut weight_sum = 0_u128;
    let mut records_valid = true;

    for stream in streams {
        records_valid &= validate_weight_record(&stream.weight).is_ok();
        let weight = compute_weight(&stream.weight, epoch);
        weight_sum = weight_sum.saturating_add(u128::from(weight));
        let mut portion = TokenAmount::from_atto(block_reward.atto() * weight / &denom);
        allocated += &portion;
        if let Some(distribution) = &stream.distribution {
            let share_total = stored_share_total(&distribution.shares)?;
            ensure!(share_total <= DENOM, "stored shares sum to {share_total}, exceeds {DENOM}");
            if share_total != DENOM {
                let service_portion = TokenAmount::from_atto(portion.atto() * share_total / &denom);
                burn += &portion - &service_portion;
                portion = service_portion;
            }
            service.push(StreamAccrual { id: stream.id, amount: portion });
        } else {
            miner += portion;
        }
    }

    let schedule_valid =
        records_valid && weight_sum <= u128::from(DENOM) && allocated <= *block_reward;
    if !schedule_valid {
        return Ok(RewardAllocation {
            miner: TokenAmount::zero(),
            service: Vec::new(),
            burn: TokenAmount::zero(),
            schedule_valid,
        });
    }

    burn += block_reward - allocated;
    Ok(RewardAllocation { miner, service, burn, schedule_valid })
}

/// Adds this award's explicit-stream portions to their matching inline accruals.
pub(crate) fn accrue_service(
    accruals: &mut [StreamAccrual],
    portions: &[StreamAccrual],
) -> Result<()> {
    let mut seen = BTreeSet::new();
    for portion in portions {
        ensure!(!portion.amount.is_negative(), "service portion is negative");
        ensure!(seen.insert(portion.id), "duplicate service portion for stream {}", portion.id);
        ensure!(
            accruals.iter().any(|row| row.id == portion.id),
            "missing accrual for stream {}",
            portion.id
        );
    }
    // The preflight above proves every lookup in this mutation pass succeeds.
    for portion in portions {
        let row = accruals
            .iter_mut()
            .find(|row| row.id == portion.id)
            .expect("explicit-stream accrual presence validated");
        row.amount += &portion.amount;
    }
    Ok(())
}

/// Computes explicit-stream funds still held by f02.
pub fn compute_service_liability(
    streams: &StreamsState,
    accruals: &[StreamAccrual],
) -> Result<TokenAmount> {
    let mut total = TokenAmount::zero();
    let mut accruals = accruals.iter();

    for stream in &streams.streams {
        let Some(distribution) = &stream.distribution else {
            // Implicit streams pay the miner directly and carry no service liability.
            continue;
        };
        let accrual = accruals
            .next()
            .ok_or_else(|| anyhow::anyhow!("missing accrual for stream {}", stream.id))?;
        ensure!(
            accrual.id == stream.id,
            "explicit-stream accrual {} does not match explicit stream {}",
            accrual.id,
            stream.id
        );
        ensure!(
            !accrual.amount.is_negative(),
            "explicit-stream accrual for stream {} is negative",
            stream.id
        );
        validate_period_claims(distribution, &accrual.amount)?;

        let claimed: TokenAmount = distribution.claimed_period.iter().map(|row| &row.amount).sum();
        total += &accrual.amount - claimed;
        total += distribution.payable.iter().map(|row| &row.amount).sum::<TokenAmount>();
    }
    if let Some(accrual) = accruals.next() {
        return Err(anyhow::anyhow!(
            "explicit-stream accrual {} has no matching explicit stream",
            accrual.id
        ));
    }
    for tombstone in &streams.tombstones {
        validate_amount_rows(&tombstone.payable, "tombstone payable")?;
        total += tombstone.payable.iter().map(|row| &row.amount).sum::<TokenAmount>();
    }
    Ok(total)
}
