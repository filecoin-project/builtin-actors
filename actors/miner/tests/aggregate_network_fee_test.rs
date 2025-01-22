use fil_actor_miner::aggregate_prove_commit_network_fee;
use fil_actor_miner::detail::BATCH_BALANCER;
use fvm_shared::econ::TokenAmount;
use num_traits::zero;

#[test]
fn constant_fee_per_sector_when_base_fee_is_below_batch_balancer() {
    let one_sector_fee = aggregate_prove_commit_network_fee(1, &zero());
    let ten_sector_fee = aggregate_prove_commit_network_fee(10, &zero());
    assert_eq!(&one_sector_fee * 10, ten_sector_fee);

    let forty_sector_fee = aggregate_prove_commit_network_fee(40, &TokenAmount::from_nano(1));
    assert_eq!(&one_sector_fee * 40, forty_sector_fee);

    let two_hundred_sector_fee =
        aggregate_prove_commit_network_fee(200, &TokenAmount::from_nano(2));
    assert_eq!(one_sector_fee * 200, two_hundred_sector_fee);
}

#[test]
fn fee_increases_if_basefee_crosses_threshold() {
    let at_no_base_fee = aggregate_prove_commit_network_fee(10, &zero());
    let at_balance_minus_one_base_fee =
        aggregate_prove_commit_network_fee(10, &(&*BATCH_BALANCER - TokenAmount::from_atto(1)));
    let at_balance_base_fee = aggregate_prove_commit_network_fee(10, &BATCH_BALANCER);
    let at_balance_plus_one_base_fee =
        aggregate_prove_commit_network_fee(10, &(&*BATCH_BALANCER + TokenAmount::from_nano(1)));
    let at_balance_plus_two_base_fee =
        aggregate_prove_commit_network_fee(10, &(&*BATCH_BALANCER + TokenAmount::from_nano(2)));
    let at_balance_times_two_base = aggregate_prove_commit_network_fee(10, &(2 * &*BATCH_BALANCER));

    assert_eq!(at_no_base_fee, at_balance_minus_one_base_fee);
    assert_eq!(at_no_base_fee, at_balance_base_fee);
    assert!(at_balance_base_fee < at_balance_plus_one_base_fee);
    assert!(at_balance_plus_one_base_fee < at_balance_plus_two_base_fee);
    assert_eq!(at_balance_times_two_base, 2 * at_balance_base_fee);
}

#[test]
fn regression_tests() {
    let magic_number = 49299973;
    let fee = |aggregate_size, base_fee_multiplier| {
        aggregate_prove_commit_network_fee(
            aggregate_size,
            &TokenAmount::from_nano(base_fee_multiplier),
        )
    };

    // Under batch balancer (2), so these two are the same:
    // 2/20 * x * 10 = (2/2) * x = x
    let expected = TokenAmount::from_nano(magic_number);
    assert_eq!(expected, fee(10, 0));
    assert_eq!(expected, fee(10, 1));

    // 3/20 * x * 100 = 15 * x
    let expected = TokenAmount::from_nano(15) * magic_number;
    assert_eq!(expected, fee(100, 3));

    // 6/20 * x * 100 = 30 * x
    let expected = TokenAmount::from_nano(30) * magic_number;
    assert_eq!(expected, fee(100, 6));
}
