use std::collections::HashMap;

use fil_actor_miner::{
    aggregate_pre_commit_network_fee, aggregate_prove_commit_network_fee,
    PreCommitSectorBatchParams, State,
};
use fil_actors_runtime::test_utils::expect_abort_contains_message;
use fvm_ipld_bitfield::BitField;
use fvm_shared::{bigint::BigInt, clock::ChainEpoch, error::ExitCode};
use lazy_static::lazy_static;

mod util;
use num_traits::Zero;
use util::*;

lazy_static! {
    static ref PERIOD_OFFSET: ChainEpoch = ChainEpoch::from(100);
}

const BIG_BALANCE: u128 = 1_000_000_000_000_000_000_000_000u128;

#[test]
fn insufficient_funds_for_aggregated_prove_commit_network_fee() {
    let actor = ActorHarness::new(*PERIOD_OFFSET);
    let mut rt = actor.new_runtime();
    rt.set_balance(BigInt::from(BIG_BALANCE));
    let precommit_epoch = *PERIOD_OFFSET + 1;
    rt.set_epoch(precommit_epoch);
    actor.construct_and_verify(&mut rt);
    let dl_info = actor.deadline(&rt);

    // make a good commitment for the proof to target
    let prove_commit_epoch = precommit_epoch + rt.policy.pre_commit_challenge_delay + 1;
    // something on deadline boundary but > 180 days
    let expiration =
        dl_info.period_end() + rt.policy.wpost_proving_period * DEFAULT_SECTOR_EXPIRATION as i64;

    let mut precommits = Vec::new();
    let mut sector_nos_bf = BitField::new();
    for i in 0..4 {
        sector_nos_bf.set(i);
        let precommit_params =
            actor.make_pre_commit_params(i, precommit_epoch - 1, expiration, vec![1]);
        let precommit = actor.pre_commit_sector_and_get(
            &mut rt,
            precommit_params,
            PreCommitConfig::empty(),
            i == 0,
        );
        precommits.push(precommit);
    }

    // set base fee extremely high so AggregateProveCommitNetworkFee is > 1000 FIL. Set balance to 1000 FIL to easily cover IP but not cover network fee
    rt.set_epoch(prove_commit_epoch);
    let balance = BigInt::from(1000u64) * BigInt::from(10u64.pow(18));
    rt.set_balance(balance.clone());
    let base_fee = BigInt::from(10u64.pow(16));
    rt.set_base_fee(base_fee.clone());
    assert!(aggregate_prove_commit_network_fee(precommits.len() as i64, &base_fee) > balance);
    // TODO: needs ActorHarness::prove_commit_aggregate_sector (in miner tests part 16)
}

#[test]
fn insufficient_funds_for_batch_precommit_network_fee() {
    let actor = ActorHarness::new(*PERIOD_OFFSET);
    let mut rt = actor.new_runtime();
    rt.set_balance(BigInt::from(BIG_BALANCE));
    let precommit_epoch = *PERIOD_OFFSET + 1;
    rt.set_epoch(precommit_epoch);
    actor.construct_and_verify(&mut rt);
    let dl_info = actor.deadline(&rt);
    // something on deadline boundary but > 180 days
    let expiration =
        dl_info.period_end() + rt.policy.wpost_proving_period * DEFAULT_SECTOR_EXPIRATION as i64;

    let mut precommits = Vec::new();
    let mut sector_nos_bf = BitField::new();
    for i in 0..4 {
        sector_nos_bf.set(i);
        let precommit = actor.make_pre_commit_params(i, precommit_epoch - 1, expiration, vec![]);
        precommits.push(precommit);
    }

    // set base fee extremely high so AggregateProveCommitNetworkFee is > 1000 FIL. Set balance to 1000 FIL to easily cover PCD but not network fee
    let balance = BigInt::from(1000u64) * BigInt::from(10u64.pow(18));
    rt.set_balance(balance.clone());
    let base_fee = BigInt::from(10u64.pow(16));
    rt.set_base_fee(base_fee.clone());
    assert!(aggregate_pre_commit_network_fee(precommits.len() as i64, &base_fee) > balance);

    let res = actor.pre_commit_sector_batch(
        &mut rt,
        PreCommitSectorBatchParams { sectors: precommits },
        &PreCommitBatchConfig { first_for_miner: true, ..Default::default() },
        base_fee,
    );

    // state untouched
    let state: State = rt.get_state();
    assert!(state.pre_commit_deposits == BigInt::zero());
    let expirations = actor.collect_precommit_expirations(&rt, &state);
    assert_eq!(HashMap::new(), expirations);

    expect_abort_contains_message(
        ExitCode::USR_INSUFFICIENT_FUNDS,
        "unlocked balance can not repay fee debt",
        res,
    );
}

#[test]
fn insufficient_funds_for_batch_precommit_in_combination_of_fee_debt_and_network_fee() {
    let actor = ActorHarness::new(*PERIOD_OFFSET);
    let mut rt = actor.new_runtime();
    rt.set_balance(BigInt::from(BIG_BALANCE));
    let precommit_epoch = *PERIOD_OFFSET + 1;
    rt.set_epoch(precommit_epoch);
    actor.construct_and_verify(&mut rt);
    let dl_info = actor.deadline(&rt);
    // something on deadline boundary but > 180 days
    let expiration =
        dl_info.period_end() + rt.policy.wpost_proving_period * DEFAULT_SECTOR_EXPIRATION as i64;

    let mut precommits = Vec::new();
    let mut sector_nos_bf = BitField::new();
    for i in 0..4 {
        sector_nos_bf.set(i);
        let precommit = actor.make_pre_commit_params(i, precommit_epoch - 1, expiration, vec![]);
        precommits.push(precommit);
    }

    // set base fee extremely high so AggregateProveCommitNetworkFee is > 1000 FIL. Set balance to 1000 FIL to easily cover PCD but not network fee
    let base_fee = BigInt::from(10u64.pow(16));
    rt.set_base_fee(base_fee.clone());
    let net_fee = aggregate_pre_commit_network_fee(precommits.len() as i64, &base_fee);

    // setup miner to have fee debt equal to net fee
    let mut state: State = rt.get_state();
    state.fee_debt = net_fee.clone();
    rt.replace_state(&state);

    // give miner almost enough balance to pay both
    let mut balance = BigInt::from(2) * net_fee;
    balance -= 1;
    rt.set_balance(balance);

    let res = actor.pre_commit_sector_batch(
        &mut rt,
        PreCommitSectorBatchParams { sectors: precommits },
        &PreCommitBatchConfig { first_for_miner: true, ..Default::default() },
        base_fee,
    );

    // state untouched
    let state: State = rt.get_state();
    assert!(state.pre_commit_deposits == BigInt::zero());
    let expirations = actor.collect_precommit_expirations(&rt, &state);
    assert_eq!(HashMap::new(), expirations);

    expect_abort_contains_message(
        ExitCode::USR_INSUFFICIENT_FUNDS,
        "unlocked balance can not repay fee debt",
        res,
    );
}

#[test]
fn enough_funds_for_fee_debt_and_network_fee_but_not_for_pcd() {}

#[test]
fn enough_funds_for_everything() {}
