use fil_actor_miner::pledge_penalty_for_continued_fault;
use fvm_shared::bigint::{BigInt, Zero};
use fvm_shared::{econ::TokenAmount, smooth::FilterEstimate};
use std::ops::Neg;

#[test]
fn zero_power_means_zero_fault_penalty() {
    let epoch_target_reward = TokenAmount::from(1_u64 << 50);
    let zero_qa_power = TokenAmount::zero();
    let network_qa_power = TokenAmount::from(1_u64 << 10);
    let power_rate_of_change = TokenAmount::from(1_u64 << 10);
    let reward_estimate = FilterEstimate::new(epoch_target_reward, BigInt::zero());
    let power_estimate = FilterEstimate::new(network_qa_power, power_rate_of_change);

    let penalty_for_zero_power_faulted =
        pledge_penalty_for_continued_fault(&reward_estimate, &power_estimate, &zero_qa_power);
    assert!(penalty_for_zero_power_faulted.is_zero());
}
