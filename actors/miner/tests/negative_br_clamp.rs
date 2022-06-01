use fil_actor_miner::expected_reward_for_power;
use fvm_shared::bigint::{BigInt, Zero};
use fvm_shared::{econ::TokenAmount, smooth::FilterEstimate};
use num_traits::sign::Signed;
use std::ops::Neg;

#[test]
fn negative_br_clamp() {
    let epoch_target_reward = TokenAmount::from(1_u64 << 50);
    let qa_sector_power = TokenAmount::from(1_u64 << 36);
    let network_qa_power = TokenAmount::from(1_u64 << 10);
    let power_rate_of_change = TokenAmount::from(1_u64 << 10).neg();
    let reward_estimate = FilterEstimate::new(epoch_target_reward, BigInt::zero());
    let power_estimate = FilterEstimate::new(network_qa_power, power_rate_of_change);
    assert!(power_estimate.estimate().is_negative());

    let four_br = expected_reward_for_power(&reward_estimate, &power_estimate, &qa_sector_power, 4);
    assert!(four_br.is_zero());
}
