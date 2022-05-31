use fil_actor_miner::aggregate_prove_commit_network_fee;
use fvm_ipld_bitfield::BitField;
use fvm_shared::{bigint::BigInt, clock::ChainEpoch};
use lazy_static::lazy_static;

mod util;
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
    let balance = BigInt::from(1000) * BigInt::from(10i32.pow(18));
    rt.set_balance(balance.clone());
    let base_fee = BigInt::from(10i32.pow(16));
    rt.set_base_fee(base_fee.clone());
    assert!(aggregate_prove_commit_network_fee(precommits.len() as i64, &base_fee) > balance);
}

#[test]
fn insufficient_funds_for_batch_precommit_network_fee() {}

#[test]
fn insufficient_funds_for_batch_precommit_in_combination_of_fee_debt_and_network_fee() {}

#[test]
fn enough_funds_for_fee_debt_and_network_fee_but_not_for_pcd() {}

#[test]
fn enough_funds_for_everything() {}
