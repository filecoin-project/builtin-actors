//! Dividing one block reward among the streams, and measuring what f02 still owes.
//!
//! FIP-0118 2.4.3, with the counters it moves specified in 2.5:
//!
//! ```text
//! AwardBlockReward(miner, penalty, gas_reward, win_count):
//!     require balance >= gas_reward
//!     load the streams block            // absent block or store error aborts
//!     computed_BR = this_epoch_reward * win_count / 5
//!     if the block is undecodable, or its stream, tombstone or queue
//!        structure is invalid:
//!         no_award()
//!     if current explicit liability is uncomputable:
//!         award_without_service()
//!     projection = project valid due writes and cancellation-stranded drops
//!     fold_dust = projection's fold dust
//!     liability =
//!         sum projected live (accrued - sum claimed_period + sum payable)
//!         + sum projected tombstone payable
//!     if liability is uncomputable:
//!         award_without_service()
//!     if balance <= gas_reward + liability + fold_dust:
//!         no_award()
//!     BR = min(computed_BR, balance - gas_reward - liability - fold_dust)
//!     evaluated = ComputeWeight for every active stream
//!     if any record violates 0 <= floor <= v_start <= cap <= DENOM
//!        or sum evaluated > DENOM:
//!         no_award()
//!     commit projection
//!     miner_reward = 0
//!     allocated = 0
//!     burn = 0
//!     for each active stream s:                     // in list order
//!         portion = floor(evaluated[s] * BR / DENOM)
//!         allocated += portion
//!         if s.distribution is IMPLICIT:
//!             miner_reward += portion
//!         else:
//!             share_total = sum s.distribution.shares // stored; f099 absent
//!             accrue = floor(portion * share_total / DENOM)
//!             accrued[s.id] += accrue
//!             total_explicit_minted += accrue
//!             burn += portion - accrue
//!     burn += BR - allocated
//!     send(f099, burn + fold_dust); total_burn_minted += burn
//!     total_minted_reward += BR
//!     pay miner_reward + gas_reward to winning miner; penalties as today
//!
//! no_award():
//!     pay gas_reward and apply penalty as today; return without state change
//!
//! award_without_service():          // stored state, never the projection
//!     ceiling = min(STORAGE_MINING_ALLOCATION - total_minted_reward,
//!                   balance - gas_reward)
//!     evaluated = ComputeWeight for every active stream
//!     if ceiling <= 0
//!        or any record violates 0 <= floor <= v_start <= cap <= DENOM
//!        or sum evaluated > DENOM:
//!         no_award()
//!     BR = min(computed_BR, ceiling)
//!     miner_reward = sum over IMPLICIT s of floor(evaluated[s] * BR / DENOM)
//!     burn = BR - miner_reward
//!     send(f099, burn); total_burn_minted += burn
//!     total_minted_reward += BR
//!     pay miner_reward + gas_reward to winning miner; penalties as today
//! ```
//!
//! `Actor::award_block_reward` drives the above sequence and performs the reserve check
//! and the sends, it also implements the degraded arm as `allocate_without_explicit`.
//! This module has the other pieces it calls:
//! - `allocate_reward` is the per-stream loop
//! - `accrue_explicit` adds the resulting portions to the inline accrual rows
//! - `explicit_liability` is the `liability` sum the reserve check subtracts, and the balance
//!   cover 2.5 requires of the counters this award moves

use std::collections::BTreeSet;

use anyhow::{Result, ensure};
use fvm_shared::bigint::BigInt;
use fvm_shared::clock::ChainEpoch;
use fvm_shared::econ::TokenAmount;
use num_traits::Zero;

use super::distribution::{stored_share_total, validate_amount_rows, validate_period_claims};
use super::weights::{compute_weight, validate_weight_record};
use super::{DENOM, Stream, StreamAccrual, StreamsState};

/// One block reward split into its destinations.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RewardAllocation {
    pub miner: TokenAmount,
    pub portions: Vec<StreamAccrual>,
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
    let mut portions = Vec::with_capacity(streams.len());
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
        if let Some(distribution) = stream.explicit() {
            let share_total = stored_share_total(&distribution.shares)?;
            ensure!(share_total <= DENOM, "stored shares sum to {share_total}, exceeds {DENOM}");
            if share_total != DENOM {
                let explicit_portion =
                    TokenAmount::from_atto(portion.atto() * share_total / &denom);
                burn += &portion - &explicit_portion;
                portion = explicit_portion;
            }
            portions.push(StreamAccrual { id: stream.id, amount: portion });
        } else {
            miner += portion;
        }
    }

    let schedule_valid =
        records_valid && weight_sum <= u128::from(DENOM) && allocated <= *block_reward;
    if !schedule_valid {
        return Ok(RewardAllocation {
            miner: TokenAmount::zero(),
            portions: Vec::new(),
            burn: TokenAmount::zero(),
            schedule_valid,
        });
    }

    burn += block_reward - allocated;
    Ok(RewardAllocation { miner, portions, burn, schedule_valid })
}

/// Adds this award's explicit-stream portions to their matching inline accruals.
pub(crate) fn accrue_explicit(
    accruals: &mut [StreamAccrual],
    portions: &[StreamAccrual],
) -> Result<()> {
    let mut seen = BTreeSet::new();
    for portion in portions {
        ensure!(!portion.amount.is_negative(), "explicit portion is negative");
        ensure!(seen.insert(portion.id), "duplicate explicit portion for stream {}", portion.id);
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
pub fn explicit_liability(
    streams: &StreamsState,
    accruals: &[StreamAccrual],
) -> Result<TokenAmount> {
    let mut total = TokenAmount::zero();
    let mut accruals = accruals.iter();

    for stream in &streams.streams {
        let Some(distribution) = stream.explicit() else {
            // Implicit streams pay the miner directly and carry no explicit liability.
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
