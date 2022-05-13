use fil_actors_runtime::{test_utils::MockRuntime, EPOCHS_IN_DAY};
use fvm_ipld_bitfield::BitField;
use fvm_shared::{bigint::BigInt, clock::ChainEpoch, sector::SectorNumber};

mod util;
use num_traits::Zero;
use util::*;

#[test]
fn valid_precommits_then_aggregate_provecommit() {
    let period_offset = ChainEpoch::from(100);
    let precommit_challenge_delay = ChainEpoch::from(150);
    let wpost_proving_period = ChainEpoch::from(EPOCHS_IN_DAY);

    let actor = ActorHarness::new(period_offset);
    let mut rt = MockRuntime::default();
    let precommit_epoch = period_offset + 1;
    rt.set_epoch(precommit_epoch);
    actor.construct_and_verify(&mut rt);
    let dl_info = actor.deadline(&rt);

    // make a good commitment for the proof to target

    let prove_commit_epoch = precommit_epoch + precommit_challenge_delay + 1;
    // something on deadline boundary but > 180 days
    let expiration = dl_info.period_end() + wpost_proving_period;
    // fill the sector with verified seals
    let sector_weight = actor.sector_size as i64 * (expiration - prove_commit_epoch);
    let deal_weight = BigInt::zero();
    let verified_deal_weight = sector_weight;

    let mut precommits = vec![];
    let mut sector_nos_bf = BitField::new();
    for i in 1..=10u64 {
        let sector_number = SectorNumber::from(i);
        sector_nos_bf.set(i);
        let precommit_params =
            actor.make_pre_commit_params(sector_number, precommit_epoch - 1, expiration, vec![1]);
        let config =
            PreCommitConfig::new(deal_weight.clone(), BigInt::from(verified_deal_weight), None);
        let precommit = actor.pre_commit_sector(&mut rt, precommit_params, config, i == 0);
        precommits.push(precommit);
    }

    // todo: flush map to run to match partition state
    // sector_nos_bf.copy() ?

    // run prove commit logic
    rt.set_epoch(prove_commit_epoch);
    rt.set_balance(BigInt::from(1000u64) * BigInt::from(1u64.pow(18)));

    actor.prove_commit_aggregate_sector(
        rt,
        ProveCommitConfig::empty(),
        precommits,
        make_prove_commit_aggregate(sector_nos_bf),
        BigInt::zero(),
    );

    // expect precommits to have been removed
    let st = actor.get_state(&rt);

    assert_eq!(BigInt::zero(),)
}
