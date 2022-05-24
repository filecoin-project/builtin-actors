use std::ops::Neg;

use fil_actor_miner::detail::expected_reward_for_power_clamped_at_atto_fil;
use fvm_shared::bigint::{BigInt, Zero};
use fvm_shared::econ::TokenAmount;
use fvm_shared::sector::StoragePower;
use fvm_shared::smooth::FilterEstimate;

#[test]
fn expected_zero_valued_br_clamped_at_1_attofil() {
    let epoch_target_reward = TokenAmount::from(1u64 << 50);
    let zero_qa_power = StoragePower::zero();
    let network_qa_power = StoragePower::from(1u64 << 10);
    let power_rate_of_change = StoragePower::from(1 << 10);
    let reward_estimate = FilterEstimate::new(epoch_target_reward, BigInt::zero());
    let power_estimate = FilterEstimate::new(network_qa_power, power_rate_of_change);

    let br_clamped = expected_reward_for_power_clamped_at_atto_fil(
        &reward_estimate,
        &power_estimate,
        &zero_qa_power,
        1,
    );
    assert_eq!(br_clamped, BigInt::from(1));
}

#[test]
fn expected_negative_value_br_clamped_at_1_atto_fil() {
    let epoch_target_reward = TokenAmount::from(1u64 << 50);
    let qa_sector_power = StoragePower::from(1u64 << 36);
    let network_qa_power = StoragePower::from(1u64 << 10);
    let power_rate_of_change = StoragePower::from(1 << 10).neg();
    let reward_estimate = FilterEstimate::new(epoch_target_reward, BigInt::zero());
    let power_estimate = FilterEstimate::new(network_qa_power, power_rate_of_change);

    let four_br_clamped = expected_reward_for_power_clamped_at_atto_fil(
        &reward_estimate,
        &power_estimate,
        &qa_sector_power,
        4,
    );
    assert_eq!(four_br_clamped, BigInt::from(1));
}
