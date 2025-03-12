use fil_actor_miner::{expected_reward_for_power, pledge_penalty_for_continued_fault};
use fil_actors_runtime::reward::FilterEstimate;
use fvm_shared::{
    bigint::{BigInt, Zero},
    econ::TokenAmount,
};
use num_traits::sign::Signed;
use std::ops::Neg;

#[test]
fn negative_br_clamp() {
    let epoch_target_reward = BigInt::from(1_u64 << 50);
    let qa_sector_power = BigInt::from(1_u64 << 36);
    let network_qa_power = BigInt::from(1_u64 << 10);
    let power_rate_of_change = BigInt::from(1_u64 << 10).neg();
    let reward_estimate = FilterEstimate::new(epoch_target_reward, BigInt::zero());
    let power_estimate = FilterEstimate::new(network_qa_power.clone(), power_rate_of_change);
    assert!(power_estimate.extrapolate(4) < network_qa_power);

    let four_br = expected_reward_for_power(&reward_estimate, &power_estimate, &qa_sector_power, 4);
    assert!(four_br.is_zero());
}

#[test]
fn zero_power_means_zero_fault_penalty() {
    let epoch_target_reward = BigInt::from(1_u64 << 50);
    let zero_qa_power = BigInt::zero();
    let network_qa_power = BigInt::from(1_u64 << 10);
    let power_rate_of_change = BigInt::from(1_u64 << 10);
    let reward_estimate = FilterEstimate::new(epoch_target_reward, BigInt::zero());
    let power_estimate = FilterEstimate::new(network_qa_power, power_rate_of_change);

    let penalty_for_zero_power_faulted =
        pledge_penalty_for_continued_fault(&reward_estimate, &power_estimate, &zero_qa_power);
    assert!(penalty_for_zero_power_faulted.is_zero());
}

// Test case introduced in FIP-0098.
// `pledge_penalty_for_continued_fault` should work for aggregate power numbers above the possible QA power for any single sector. For instance, an aggregate of 10 sectors' power should return the same end result as summing the `pledge_penalty_for_continued_fault` of each sector individually.
#[test]
fn aggregate_power_pledge_penalty_for_continued_fault() {
    let epoch_target_reward = BigInt::from(1_u64 << 50);
    let network_qa_power = BigInt::from(1_u64 << 10);
    let power_rate_of_change = BigInt::from(1_u64 << 10);
    let reward_estimate = FilterEstimate::new(epoch_target_reward, BigInt::zero());
    let power_estimate = FilterEstimate::new(network_qa_power, power_rate_of_change);

    // Note: these cases follow a discussion in
    // [FIP-0098](https://github.com/filecoin-project/FIPs/pull/1128#discussion_r1978683778)
    let cases = [
        (10, BigInt::from(1u64 << 6)),
        (10, BigInt::from(1u64 << 36)),
        (10, BigInt::from(1u64 << 50)),
        (1000, BigInt::from(1u64 << 6)),
        (1000, BigInt::from(1u64 << 36)),
        (1000, BigInt::from(1u64 << 50)),
    ];

    for (sector_multiple, qa_power) in cases.into_iter() {
        // allow 1 atto per sector difference
        let allowed_atto_difference = BigInt::from(sector_multiple);
        let aggregate_penalty = pledge_penalty_for_continued_fault(
            &reward_estimate,
            &power_estimate,
            &(&qa_power * sector_multiple),
        );
        let individual_penalties: TokenAmount = sector_multiple
            * pledge_penalty_for_continued_fault(&reward_estimate, &power_estimate, &qa_power);
        assert!(aggregate_penalty > TokenAmount::zero());
        println!(
            "aggregate_penalty: {}, individual_penalties: {}, diff: {}",
            aggregate_penalty.atto(),
            individual_penalties.atto(),
            (aggregate_penalty.atto() - individual_penalties.atto()).abs()
        );
        assert!(
            (aggregate_penalty.atto() - individual_penalties.atto()).abs()
                <= allowed_atto_difference
        );
    }
}
