// Copyright 2019-2022 ChainSafe Systems
// SPDX-License-Identifier: Apache-2.0, MIT

use std::cmp;

use fil_actors_runtime::EXPECTED_LEADERS_PER_EPOCH;
use fil_actors_runtime::network::EPOCHS_IN_DAY;
use fil_actors_runtime::reward::math::PRECISION;
use fil_actors_runtime::reward::{FilterEstimate, smooth};
use fvm_shared::bigint::Integer;
use fvm_shared::clock::ChainEpoch;
use fvm_shared::econ::TokenAmount;
use fvm_shared::sector::StoragePower;
use lazy_static::lazy_static;
use num_traits::Zero;

use super::{REWARD_VESTING_SPEC, VestSpec};
use crate::detail::*;

/// Projection period of expected sector block reward for deposit required to pre-commit a sector.
/// This deposit is lost if the pre-commitment is not timely followed up by a commitment proof.
const PRE_COMMIT_DEPOSIT_FACTOR: u64 = 20;

/// Projection period of expected sector block rewards for storage pledge required to commit a sector.
/// This pledge is lost if a sector is terminated before its full committed lifetime.
pub const INITIAL_PLEDGE_FACTOR: u64 = 20;

pub const PRE_COMMIT_DEPOSIT_PROJECTION_PERIOD: i64 =
    (PRE_COMMIT_DEPOSIT_FACTOR as ChainEpoch) * EPOCHS_IN_DAY;
pub const INITIAL_PLEDGE_PROJECTION_PERIOD: i64 =
    (INITIAL_PLEDGE_FACTOR as ChainEpoch) * EPOCHS_IN_DAY;

const LOCK_TARGET_FACTOR_NUM: u32 = 3;
const LOCK_TARGET_FACTOR_DENOM: u32 = 10;

pub const TERMINATION_REWARD_FACTOR_NUM: u32 = 1;
pub const TERMINATION_REWARD_FACTOR_DENOM: u32 = 2;

// * go impl has 75/100 but this is just simplified
const LOCKED_REWARD_FACTOR_NUM: u32 = 3;
const LOCKED_REWARD_FACTOR_DENOM: u32 = 4;

/// Used to compute termination fees in the base case by multiplying against initial pledge.
pub const TERM_FEE_PLEDGE_MULTIPLE_NUM: u32 = 85;
pub const TERM_FEE_PLEDGE_MULTIPLE_DENOM: u32 = 1000;

/// Used to ensure the termination fee for young sectors is not arbitrarily low.
pub const TERM_FEE_MIN_PLEDGE_MULTIPLE_NUM: u32 = 2;
pub const TERM_FEE_MIN_PLEDGE_MULTIPLE_DENOM: u32 = 100;

/// Used to compute termination fees when the termination fee of a sector is less than the fault fee for the same sector.
pub const TERM_FEE_MAX_FAULT_FEE_MULTIPLE_NUM: u32 = 105;
pub const TERM_FEE_MAX_FAULT_FEE_MULTIPLE_DENOM: u32 = 100;

lazy_static! {
    /// Cap on initial pledge requirement for sectors during the Space Race network.
    /// The target is 1 FIL (10**18 attoFIL) per 32GiB.
    /// This does not divide evenly, so the result is fractionally smaller.
    static ref INITIAL_PLEDGE_MAX_PER_BYTE: TokenAmount =
        TokenAmount::from_whole(1).div_floor(32i64 << 30);

    /// Base reward for successfully disputing a window posts proofs.
    pub static ref BASE_REWARD_FOR_DISPUTED_WINDOW_POST: TokenAmount = TokenAmount::from_whole(4);

    /// Base penalty for a successful disputed window post proof.
    pub static ref BASE_PENALTY_FOR_DISPUTED_WINDOW_POST: TokenAmount = TokenAmount::from_whole(20);
}
// FF + 2BR
const INVALID_WINDOW_POST_PROJECTION_PERIOD: ChainEpoch =
    CONTINUED_FAULT_PROJECTION_PERIOD + 2 * EPOCHS_IN_DAY;

// Projection period of expected daily sector block reward penalised when a fault is continued after initial detection.
// This guarantees that a miner pays back at least the expected block reward earned since the last successful PoSt.
// The network conservatively assumes the sector was faulty since the last time it was proven.
// This penalty is currently overly punitive for continued faults.
// FF = BR(t, ContinuedFaultProjectionPeriod)
const CONTINUED_FAULT_FACTOR_NUM: i64 = 351;
const CONTINUED_FAULT_FACTOR_DENOM: i64 = 100;
pub const CONTINUED_FAULT_PROJECTION_PERIOD: ChainEpoch =
    (EPOCHS_IN_DAY * CONTINUED_FAULT_FACTOR_NUM) / CONTINUED_FAULT_FACTOR_DENOM;

const TERMINATION_PENALTY_LOWER_BOUND_PROJECTIONS_PERIOD: ChainEpoch = (EPOCHS_IN_DAY * 35) / 10;

// Maximum number of lifetime days penalized when a sector is terminated.
pub const TERMINATION_LIFETIME_CAP: ChainEpoch = 140;

// Multiplier of whole per-winner rewards for a consensus fault penalty.
const CONSENSUS_FAULT_FACTOR: u64 = 5;

const GAMMA_FIXED_POINT_FACTOR: u64 = 1000; // 3 decimal places

/// The projected block reward a sector would earn over some period.
/// Also known as "BR(t)".
/// BR(t) = ProjectedRewardFraction(t) * SectorQualityAdjustedPower
/// ProjectedRewardFraction(t) is the sum of estimated reward over estimated total power
/// over all epochs in the projection period [t t+projectionDuration]
pub fn expected_reward_for_power(
    reward_estimate: &FilterEstimate,
    network_qa_power_estimate: &FilterEstimate,
    qa_sector_power: &StoragePower,
    projection_duration: ChainEpoch,
) -> TokenAmount {
    let network_qa_power_smoothed = network_qa_power_estimate.estimate();

    if network_qa_power_smoothed.is_zero() {
        return TokenAmount::from_atto(reward_estimate.estimate());
    }

    let expected_reward_for_proving_period = smooth::extrapolated_cum_sum_of_ratio(
        projection_duration,
        0,
        reward_estimate,
        network_qa_power_estimate,
    );
    let br128 = qa_sector_power * expected_reward_for_proving_period; // Q.0 * Q.128 => Q.128
    TokenAmount::from_atto(std::cmp::max(br128 >> PRECISION, Default::default()))
}

pub mod detail {
    use super::*;

    // BR but zero values are clamped at 1 attofil
    // Some uses of BR (PCD, IP) require a strictly positive value for BR derived values so
    // accounting variables can be used as succinct indicators of miner activity.
    pub fn expected_reward_for_power_clamped_at_atto_fil(
        reward_estimate: &FilterEstimate,
        network_qa_power_estimate: &FilterEstimate,
        qa_sector_power: &StoragePower,
        projection_duration: ChainEpoch,
    ) -> TokenAmount {
        let br = expected_reward_for_power(
            reward_estimate,
            network_qa_power_estimate,
            qa_sector_power,
            projection_duration,
        );
        if br.le(&TokenAmount::zero()) { TokenAmount::from_atto(1) } else { br }
    }
}

// func ExpectedRewardForPowerClampedAtAttoFIL(rewardEstimate, networkQAPowerEstimate smoothing.FilterEstimate, qaSectorPower abi.StoragePower, projectionDuration abi.ChainEpoch) abi.TokenAmount {
// 	br := ExpectedRewardForPower(rewardEstimate, networkQAPowerEstimate, qaSectorPower, projectionDuration)
// 	if br.LessThanEqual(big.Zero()) {
// 		br = abi.NewTokenAmount(1)
// 	}
// 	return br
// }

/// The penalty for a sector continuing faulty for another proving period.
/// It is a projection of the expected reward earned by the sector.
/// Also known as "FF(t)"
pub fn pledge_penalty_for_continued_fault(
    reward_estimate: &FilterEstimate,
    network_qa_power_estimate: &FilterEstimate,
    qa_sector_power: &StoragePower,
) -> TokenAmount {
    expected_reward_for_power(
        reward_estimate,
        network_qa_power_estimate,
        qa_sector_power,
        CONTINUED_FAULT_PROJECTION_PERIOD,
    )
}

/// This is the SP(t) penalty for a newly faulty sector that has not been declared.
/// SP(t) = UndeclaredFaultFactor * BR(t)
pub fn pledge_penalty_for_termination_lower_bound(
    reward_estimate: &FilterEstimate,
    network_qa_power_estimate: &FilterEstimate,
    qa_sector_power: &StoragePower,
) -> TokenAmount {
    expected_reward_for_power(
        reward_estimate,
        network_qa_power_estimate,
        qa_sector_power,
        TERMINATION_PENALTY_LOWER_BOUND_PROJECTIONS_PERIOD,
    )
}

/// Calculates termination fee for a given sector. Normally, it's calculated as a fixed percentage
/// of the initial pledge. However, there are some special cases outlined in the
/// [FIP-0098](https://github.com/filecoin-project/FIPs/blob/master/FIPS/fip-0098.md).
pub fn pledge_penalty_for_termination(
    initial_pledge: &TokenAmount,
    sector_age: ChainEpoch,
    fault_fee: &TokenAmount,
) -> TokenAmount {
    // Use the _percentage of the initial pledge_ strategy to determine the termination fee.
    let simple_termination_fee =
        (initial_pledge * TERM_FEE_PLEDGE_MULTIPLE_NUM).div_floor(TERM_FEE_PLEDGE_MULTIPLE_DENOM);

    let duration_termination_fee =
        (sector_age * &simple_termination_fee).div_floor(TERMINATION_LIFETIME_CAP * EPOCHS_IN_DAY);

    // Apply the age adjustment for young sectors to arrive at the base termination fee.
    let base_termination_fee = cmp::min(simple_termination_fee, duration_termination_fee);

    // Calculate the minimum allowed fee (a lower bound on the termination fee) by comparing the absolute minimum termination fee value against the fault fee. Whatever result is _larger_ sets the lower bound for the termination fee.
    let minimum_fee_abs = (initial_pledge * TERM_FEE_MIN_PLEDGE_MULTIPLE_NUM)
        .div_floor(TERM_FEE_MIN_PLEDGE_MULTIPLE_DENOM);
    let minimum_fee_ff = (fault_fee * TERM_FEE_MAX_FAULT_FEE_MULTIPLE_NUM)
        .div_floor(TERM_FEE_MAX_FAULT_FEE_MULTIPLE_DENOM);
    let minimum_fee = cmp::max(minimum_fee_abs, minimum_fee_ff);

    cmp::max(base_termination_fee, minimum_fee)
}

// The penalty for optimistically proving a sector with an invalid window PoSt.
pub fn pledge_penalty_for_invalid_windowpost(
    reward_estimate: &FilterEstimate,
    network_qa_power_estimate: &FilterEstimate,
    qa_sector_power: &StoragePower,
) -> TokenAmount {
    expected_reward_for_power(
        reward_estimate,
        network_qa_power_estimate,
        qa_sector_power,
        INVALID_WINDOW_POST_PROJECTION_PERIOD,
    ) + &*BASE_PENALTY_FOR_DISPUTED_WINDOW_POST
}

/// Computes the PreCommit deposit given sector qa weight and current network conditions.
/// PreCommit Deposit = BR(PreCommitDepositProjectionPeriod)
pub fn pre_commit_deposit_for_power(
    reward_estimate: &FilterEstimate,
    network_qa_power_estimate: &FilterEstimate,
    qa_sector_power: &StoragePower,
) -> TokenAmount {
    expected_reward_for_power_clamped_at_atto_fil(
        reward_estimate,
        network_qa_power_estimate,
        qa_sector_power,
        PRE_COMMIT_DEPOSIT_PROJECTION_PERIOD,
    )
}

/// Computes the pledge requirement for committing new quality-adjusted power to the network, given
/// the current network total and baseline power, per-epoch reward, and circulating token supply.
/// The pledge comprises two parts:
/// - storage pledge, aka IP base: a multiple of the reward expected to be earned by newly-committed power
/// - consensus pledge, aka additional IP: a pro-rata fraction of the circulating money supply
///
/// IP = IPBase(t) + AdditionalIP(t)
/// IPBase(t) = BR(t, InitialPledgeProjectionPeriod)
/// AdditionalIP(t) = LockTarget(t)*PledgeShare(t)
/// LockTarget = (LockTargetFactorNum / LockTargetFactorDenom) * FILCirculatingSupply(t)
/// PledgeShare(t) = sectorQAPower / max(BaselinePower(t), NetworkQAPower(t))
pub fn initial_pledge_for_power(
    qa_power: &StoragePower,
    baseline_power: &StoragePower,
    reward_estimate: &FilterEstimate,
    network_qa_power_estimate: &FilterEstimate,
    circulating_supply: &TokenAmount,
    epochs_since_ramp_start: i64,
    ramp_duration_epochs: u64,
) -> TokenAmount {
    let ip_base = expected_reward_for_power_clamped_at_atto_fil(
        reward_estimate,
        network_qa_power_estimate,
        qa_power,
        INITIAL_PLEDGE_PROJECTION_PERIOD,
    );

    let lock_target_num = circulating_supply.atto() * LOCK_TARGET_FACTOR_NUM;
    let lock_target_denom = LOCK_TARGET_FACTOR_DENOM;
    let pledge_share_num = qa_power;
    let network_qa_power = network_qa_power_estimate.estimate();

    // Once FIP-0081 has fully activated, additional pledge will be 70% baseline
    // pledge + 30% simple pledge.
    const FIP_0081_ACTIVATION_PERMILLE: i64 = 300;
    // Gamma/GAMMA_FIXED_POINT_FACTOR is the share of pledge coming from the
    // baseline formulation, with 1-(gamma/GAMMA_FIXED_POINT_FACTOR) coming from
    // simple pledge.
    // gamma = 1000 - 300 * (epochs_since_ramp_start / ramp_duration_epochs).max(0).min(1)
    let skew = if epochs_since_ramp_start < 0 {
        // No skew before ramp start
        0
    } else if ramp_duration_epochs == 0 || epochs_since_ramp_start >= ramp_duration_epochs as i64 {
        // 100% skew after ramp end
        FIP_0081_ACTIVATION_PERMILLE as u64
    } else {
        ((epochs_since_ramp_start * FIP_0081_ACTIVATION_PERMILLE) / ramp_duration_epochs as i64)
            as u64
    };
    let gamma = GAMMA_FIXED_POINT_FACTOR - skew;

    let additional_ip_num = lock_target_num * pledge_share_num;

    let pledge_share_denom_baseline =
        cmp::max(cmp::max(&network_qa_power, baseline_power), qa_power);
    let pledge_share_denom_simple = cmp::max(&network_qa_power, qa_power);

    let additional_ip_denom_baseline = pledge_share_denom_baseline * lock_target_denom;
    let additional_ip_baseline = (gamma * &additional_ip_num)
        .div_floor(&(additional_ip_denom_baseline * GAMMA_FIXED_POINT_FACTOR));
    let additional_ip_denom_simple = pledge_share_denom_simple * lock_target_denom;
    let additional_ip_simple = ((GAMMA_FIXED_POINT_FACTOR - gamma) * &additional_ip_num)
        .div_floor(&(additional_ip_denom_simple * GAMMA_FIXED_POINT_FACTOR));

    // convex combination of simple and baseline pledge
    let additional_ip = additional_ip_baseline + additional_ip_simple;

    let nominal_pledge = ip_base + TokenAmount::from_atto(additional_ip);
    let pledge_cap = TokenAmount::from_atto(INITIAL_PLEDGE_MAX_PER_BYTE.atto() * qa_power);

    cmp::min(nominal_pledge, pledge_cap)
}

pub fn consensus_fault_penalty(this_epoch_reward: TokenAmount) -> TokenAmount {
    (this_epoch_reward * CONSENSUS_FAULT_FACTOR).div_floor(EXPECTED_LEADERS_PER_EPOCH)
}

/// Returns the amount of a reward to vest, and the vesting schedule, for a reward amount.
pub fn locked_reward_from_reward(reward: TokenAmount) -> (TokenAmount, &'static VestSpec) {
    let lock_amount = (reward * LOCKED_REWARD_FACTOR_NUM).div_floor(LOCKED_REWARD_FACTOR_DENOM);
    (lock_amount, &REWARD_VESTING_SPEC)
}
