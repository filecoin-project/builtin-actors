use fil_actor_miner::{
    TERMINATION_LIFETIME_CAP, expected_reward_for_power, pledge_penalty_for_continued_fault,
    pledge_penalty_for_invalid_windowpost, pledge_penalty_for_termination,
};
use fil_actor_reward::DENOM;
use fil_actors_runtime::network::EPOCHS_IN_DAY;
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

    let penalty_for_zero_power_faulted = pledge_penalty_for_continued_fault(
        &reward_estimate,
        &power_estimate,
        &zero_qa_power,
        DENOM,
    );
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
            DENOM,
        );
        let individual_penalties: TokenAmount = sector_multiple
            * pledge_penalty_for_continued_fault(
                &reward_estimate,
                &power_estimate,
                &qa_power,
                DENOM,
            );
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

// FIP-0122 scales the projected block reward and keeps the fixed dispute penalty.
#[test]
fn penalties_scale_only_projected_rewards() {
    let reward_estimate = FilterEstimate::new(BigInt::from(1), BigInt::zero());
    let power_estimate = FilterEstimate::new(BigInt::from(1), BigInt::zero());
    let qa_power = BigInt::from(1);

    // At one attoFIL per epoch and full network power, the projection periods
    // yield 10,108 attoFIL for continued faults and 15,868 for invalid PoSt.
    // The 65% case floors fractional attoFIL; a weight of 1 rounds both to zero.
    let cases = [(0, 0, 0), (DENOM, 10_108, 15_868), (DENOM / 100 * 65, 6_570, 10_314), (1, 0, 0)];
    for (weight, expected_fault, expected_dispute_reward) in cases {
        let fault_fee = pledge_penalty_for_continued_fault(
            &reward_estimate,
            &power_estimate,
            &qa_power,
            weight,
        );
        assert_eq!(TokenAmount::from_atto(expected_fault), fault_fee, "weight {weight}");
        let dispute_penalty = pledge_penalty_for_invalid_windowpost(
            &reward_estimate,
            &power_estimate,
            &qa_power,
            weight,
        );
        assert_eq!(
            TokenAmount::from_whole(20) + TokenAmount::from_atto(expected_dispute_reward),
            dispute_penalty,
            "weight {weight}",
        );
    }
}

#[test]
fn penalty_scaling_preserves_big_integer_precision() {
    let epoch_reward: BigInt = BigInt::from(1) << 200;
    let reward_estimate = FilterEstimate::new(epoch_reward.clone(), BigInt::zero());
    let power_estimate = FilterEstimate::new(BigInt::from(1), BigInt::zero());
    let qa_power = BigInt::from(1);

    // The projected reward and its weighted product both exceed 128 bits.
    let fault_fee =
        pledge_penalty_for_continued_fault(&reward_estimate, &power_estimate, &qa_power, DENOM / 4);
    assert_eq!(TokenAmount::from_atto(&epoch_reward * 2_527), fault_fee);
    let dispute_penalty = pledge_penalty_for_invalid_windowpost(
        &reward_estimate,
        &power_estimate,
        &qa_power,
        DENOM / 4,
    );
    assert_eq!(
        TokenAmount::from_atto(epoch_reward * 3_967) + TokenAmount::from_whole(20),
        dispute_penalty,
    );
}

#[test]
fn termination_uses_scaled_fault_fee_and_preserves_pledge_bounds() {
    let reward_estimate = FilterEstimate::new(BigInt::from(1), BigInt::zero());
    let power_estimate = FilterEstimate::new(BigInt::from(1), BigInt::zero());
    let qa_power = BigInt::from(1);
    let initial_pledge = TokenAmount::from_atto(10_000);
    let max_age = TERMINATION_LIFETIME_CAP * EPOCHS_IN_DAY;

    // At zero weight, the 2% pledge minimum and age-adjusted 8.5% fee remain.
    // Otherwise the 105% bound on the scaled fault fee dominates these inputs.
    let cases = [
        (0, 0, 200),
        (0, max_age, 850),
        (DENOM / 100 * 65, max_age, 6_898),
        (DENOM, max_age, 10_613),
    ];
    for (weight, age, expected) in cases {
        let fault_fee = pledge_penalty_for_continued_fault(
            &reward_estimate,
            &power_estimate,
            &qa_power,
            weight,
        );
        let penalty = pledge_penalty_for_termination(&initial_pledge, age, &fault_fee);
        assert_eq!(TokenAmount::from_atto(expected), penalty, "weight {weight}, age {age}");
    }
}
