use fil_actor_miner::{
    aggregate_prove_commit_network_fee, pre_commit_deposit_for_power, qa_power_max,
    PreCommitSectorBatchParams, State,
};
use fil_actors_runtime::test_utils::expect_abort;
use fvm_ipld_bitfield::BitField;
use fvm_shared::{clock::ChainEpoch, econ::TokenAmount, error::ExitCode};
use lazy_static::lazy_static;

mod util;
use util::*;

lazy_static! {
    static ref PERIOD_OFFSET: ChainEpoch = ChainEpoch::from(100);
}

#[test]
fn insufficient_funds_for_aggregated_prove_commit_network_fee() {
    let actor = ActorHarness::new(*PERIOD_OFFSET);
    let rt = actor.new_runtime();
    rt.set_balance(BIG_BALANCE.clone());
    let precommit_epoch = *PERIOD_OFFSET + 1;
    rt.set_epoch(precommit_epoch);
    actor.construct_and_verify(&rt);
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
            &rt,
            precommit_params,
            PreCommitConfig::empty(),
            i == 0,
        );
        precommits.push(precommit);
    }

    // set base fee extremely high so AggregateProveCommitNetworkFee is > 1000 FIL. Set balance to 1000 FIL to easily cover IP but not cover network fee
    rt.set_epoch(prove_commit_epoch);
    let balance = TokenAmount::from_whole(1000);
    rt.set_balance(balance.clone());
    let base_fee = TokenAmount::from_atto(10u64.pow(16));
    rt.base_fee.replace(base_fee.clone());
    assert!(aggregate_prove_commit_network_fee(precommits.len(), &base_fee) > balance);

    let res = actor.prove_commit_aggregate_sector(
        &rt,
        ProveCommitConfig::empty(),
        precommits,
        make_prove_commit_aggregate(&sector_nos_bf),
        &base_fee,
    );

    expect_abort(ExitCode::USR_INSUFFICIENT_FUNDS, res);
    rt.reset();
}

#[test]
fn enough_funds_for_everything() {
    let actor = ActorHarness::new(*PERIOD_OFFSET);
    let rt = actor.new_runtime();
    rt.set_balance(BIG_BALANCE.clone());
    let precommit_epoch = *PERIOD_OFFSET + 1;
    rt.set_epoch(precommit_epoch);
    actor.construct_and_verify(&rt);
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

    // give miner enough balance to pay pcd
    let expected_deposit = pre_commit_deposit_for_power(
        &actor.epoch_reward_smooth,
        &actor.epoch_qa_power_smooth,
        &qa_power_max(actor.sector_size),
    ) * precommits.len();
    let balance = expected_deposit.clone();
    rt.set_balance(balance);

    actor
        .pre_commit_sector_batch(
            &rt,
            PreCommitSectorBatchParams { sectors: precommits },
            &PreCommitBatchConfig { first_for_miner: true, ..Default::default() },
        )
        .unwrap();

    // state updated
    let state: State = rt.get_state();
    assert_eq!(expected_deposit, state.pre_commit_deposits);
    let expirations = actor.collect_precommit_expirations(&rt, &state);
    assert_eq!(1, expirations.len());
    for (_, map) in expirations {
        assert_eq!(4, map.len());
    }
}
