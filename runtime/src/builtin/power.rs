// SPDX-License-Identifier: Apache-2.0, MIT

use fvm_shared::{clock::ChainEpoch, econ::TokenAmount, sector::StoragePower};
use num_traits::Zero;
use std::cmp::{self};

use super::{
    reward::{math::PRECISION, smooth::extrapolated_cum_sum_of_ratio, FilterEstimate},
    EPOCHS_IN_DAY,
};

// Projection period of expected daily sector block reward penalised when a fault is continued after initial detection.
// This guarantees that a miner pays back at least the expected block reward earned since the last successful PoSt.
// The network conservatively assumes the sector was faulty since the last time it was proven.
// This penalty is currently overly punitive for continued faults.
// FF = BR(t, ContinuedFaultProjectionPeriod)
const CONTINUED_FAULT_FACTOR_NUM: i64 = 351;
const CONTINUED_FAULT_FACTOR_DENOM: i64 = 100;
pub const CONTINUED_FAULT_PROJECTION_PERIOD: ChainEpoch =
    (EPOCHS_IN_DAY * CONTINUED_FAULT_FACTOR_NUM) / CONTINUED_FAULT_FACTOR_DENOM;

// Maximum number of lifetime days penalized when a sector is terminated.
pub const TERMINATION_LIFETIME_CAP: ChainEpoch = 140;

/// Used to compute termination fees in the base case by multiplying against initial pledge.
pub const TERM_FEE_PLEDGE_MULTIPLE_NUM: u32 = 85;
pub const TERM_FEE_PLEDGE_MULTIPLE_DENOM: u32 = 1000;

/// Used to ensure the termination fee for young sectors is not arbitrarily low.
pub const TERM_FEE_MIN_PLEDGE_MULTIPLE_NUM: u32 = 2;
pub const TERM_FEE_MIN_PLEDGE_MULTIPLE_DENOM: u32 = 100;

/// Used to compute termination fees when the termination fee of a sector is less than the fault fee for the same sector.
pub const TERM_FEE_MAX_FAULT_FEE_MULTIPLE_NUM: u32 = 105;
pub const TERM_FEE_MAX_FAULT_FEE_MULTIPLE_DENOM: u32 = 100;

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

    // Apply the age adjustment for young sectors to arrive at the base termination fee.
    let base_termination_fee = cmp::min(
        simple_termination_fee.clone(),
        (sector_age * &simple_termination_fee).div_floor(TERMINATION_LIFETIME_CAP * EPOCHS_IN_DAY),
    );

    // Calculate the minimum allowed fee (a lower bound on the termination fee) by comparing the absolute minimum termination fee value against the fault fee. Whatever result is _larger_ sets the lower bound for the termination fee.
    let minimum_fee_abs = (initial_pledge * TERM_FEE_MIN_PLEDGE_MULTIPLE_NUM)
        .div_floor(TERM_FEE_MIN_PLEDGE_MULTIPLE_DENOM);
    let minimum_fee_ff = (fault_fee * TERM_FEE_MAX_FAULT_FEE_MULTIPLE_NUM)
        .div_floor(TERM_FEE_MAX_FAULT_FEE_MULTIPLE_DENOM);
    let minimum_fee = cmp::max(minimum_fee_abs, minimum_fee_ff);

    cmp::max(base_termination_fee, minimum_fee)
}

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

    let expected_reward_for_proving_period = extrapolated_cum_sum_of_ratio(
        projection_duration,
        0,
        reward_estimate,
        network_qa_power_estimate,
    );
    let br128 = qa_sector_power * expected_reward_for_proving_period; // Q.0 * Q.128 => Q.128
    TokenAmount::from_atto(std::cmp::max(br128 >> PRECISION, Default::default()))
}
