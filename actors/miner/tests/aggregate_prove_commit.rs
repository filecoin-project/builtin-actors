use fil_actor_miner::{
    initial_pledge_for_power, qa_power_for_weight, PowerPair, QUALITY_BASE_MULTIPLIER,
    VERIFIED_DEAL_WEIGHT_MULTIPLIER,
};
use fil_actors_runtime::{runtime::Runtime, test_utils::MockRuntime, EPOCHS_IN_DAY};
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
        &rt,
        ProveCommitConfig::empty(),
        precommits,
        make_prove_commit_aggregate(&sector_nos_bf),
        BigInt::zero(),
    );

    // expect precommits to have been removed
    let st = actor.get_state(&rt);

    // todo: line 1142 in miner commitment_tests.go
    // require.NoError(t, sectorNosBf.ForEach(func(sectorNo uint64) error {
    // 	_, found, err := st.GetPrecommittedSector(rt.AdtStore(), abi.SectorNumber(sectorNo))
    // 	require.False(t, found)
    // 	return err
    // }))

    for sector_no in sector_nos_bf.iter() {
        let found = st.get_precommitted_sector(rt.store(), SectorNumber::from(sector_no)).unwrap();
    }

    // expect deposit to have been transferred to initial pledges
    assert_eq!(BigInt::zero(), st.pre_commit_deposits);

    // The sector is exactly full with verified deals, so expect fully verified power.
    // todo: lazy_static! with big ints does not play well with casts
    let expected_power = BigInt::from(actor.sector_size as i64) * (100 / 10);
    let qa_power = qa_power_for_weight(
        actor.sector_size,
        expiration - rt.epoch,
        &deal_weight,
        &BigInt::from(verified_deal_weight),
    );
    assert_eq!(expected_power, qa_power);
    let expected_initial_pledge = initial_pledge_for_power(
        &qa_power,
        &actor.baseline_power,
        &actor.epoch_reward_smooth,
        &actor.epoch_qa_power_smooth,
        &rt.total_fil_circ_supply(),
    );
    let ten_sectors_initial_pledge = BigInt::from(10) * expected_initial_pledge.clone();
    assert_eq!(ten_sectors_initial_pledge, expected_initial_pledge);

    // expect new onchain sector
    // todo: line 1161 - 1182

    let sector_power = new_power_pair(BigInt::from(actor.sector_size as i64), qa_power);
    let ten_sectors_power = new_power_pair(10 * sector_power.raw, 10 * sector_power.qa);

    let dl_idx = 0;
    let p_idx = 0;

    let (deadline, partition) = actor.get_deadline_and_partition(&rt, dl_idx, p_idx);
    assert_eq!(10, deadline.live_sectors);

    assert!(deadline.partitions_posted.is_empty());
    assert!(deadline.early_terminations.is_empty());
}
