use std::ops::{Add, Mul, Sub};

use fil_actor_miner::detail::BATCH_BALANCER;
use fil_actor_miner::{aggregate_pre_commit_network_fee, aggregate_prove_commit_network_fee};
use fil_actors_runtime::ONE_NANO_FIL;
use fvm_shared::bigint::BigInt;
use num_traits::zero;

#[test]
fn constant_fee_per_sector_when_base_fee_is_below_5_nfil() {
    for fee_func in [aggregate_prove_commit_network_fee, aggregate_pre_commit_network_fee] {
        let one_sector_fee = fee_func(1, &zero());
        let ten_sector_fee = fee_func(10, &zero());
        assert_eq!(&one_sector_fee * 10, ten_sector_fee);

        let forty_sector_fee = fee_func(40, &ONE_NANO_FIL.into());
        assert_eq!(&one_sector_fee * 40, forty_sector_fee);

        let two_hundred_sector_fee = fee_func(200, &BigInt::from(ONE_NANO_FIL * 3));
        assert_eq!(one_sector_fee * 200, two_hundred_sector_fee);
    }
}

#[test]
fn fee_increases_if_basefee_crosses_threshold() {
    for fee_func in [aggregate_prove_commit_network_fee, aggregate_pre_commit_network_fee] {
        let at_no_base_fee = fee_func(10, &zero());
        let at_balance_minus_one_base_fee = fee_func(10, &BATCH_BALANCER.to_owned().sub(1));
        let at_balance_base_fee = fee_func(10, &BATCH_BALANCER);
        let at_balance_plus_one_base_fee =
            fee_func(10, &BATCH_BALANCER.to_owned().add(ONE_NANO_FIL));
        let at_balance_plus_two_base_fee =
            fee_func(10, &BATCH_BALANCER.to_owned().add(ONE_NANO_FIL * 2));
        let at_balance_times_two_base = fee_func(10, &BATCH_BALANCER.to_owned().mul(2));

        assert_eq!(at_no_base_fee, at_balance_minus_one_base_fee);
        assert_eq!(at_no_base_fee, at_balance_base_fee);
        assert!(at_balance_base_fee < at_balance_plus_one_base_fee);
        assert!(at_balance_plus_one_base_fee < at_balance_plus_two_base_fee);
        assert_eq!(at_balance_times_two_base, 2 * at_balance_base_fee);
    }
}

#[test]
fn regression_tests() {
    let magic_number = 65733297;
    let fee = |aggregate_size, base_fee_multiplier| {
        aggregate_prove_commit_network_fee(
            aggregate_size,
            &BigInt::from(ONE_NANO_FIL * base_fee_multiplier),
        ) + aggregate_pre_commit_network_fee(
            aggregate_size,
            &BigInt::from(ONE_NANO_FIL * base_fee_multiplier),
        )
    };

    // (5/20) * x * 10 = (5/2) * x
    let expected = (ONE_NANO_FIL * 5 * magic_number) / 2;
    assert_eq!(BigInt::from(expected), fee(10, 0));
    assert_eq!(BigInt::from(expected), fee(10, 1));

    let expected = ONE_NANO_FIL * 25 * magic_number;
    assert_eq!(BigInt::from(expected), fee(100, 3));

    let expected = ONE_NANO_FIL * 30 * magic_number;
    assert_eq!(BigInt::from(expected), fee(100, 6));
}

#[test]
fn split_25_75() {
    // check 25/75% split up to uFIL precision
    let one_micro_fil = BigInt::from(ONE_NANO_FIL) * 1000;

    for base_fee_multiplier in [0, 5, 20] {
        for aggregate_size in [13, 303] {
            let fee_pre = aggregate_pre_commit_network_fee(
                aggregate_size,
                &BigInt::from(ONE_NANO_FIL * base_fee_multiplier),
            ) / &one_micro_fil;
            let fee_prove = aggregate_prove_commit_network_fee(
                aggregate_size,
                &BigInt::from(ONE_NANO_FIL * base_fee_multiplier),
            ) / &one_micro_fil;
            assert_eq!(fee_prove, 3 * fee_pre);
        }
    }
}
