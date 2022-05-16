use std::collections::HashMap;

use fil_actor_miner::{initial_pledge_for_power, qa_power_for_weight};
use fil_actors_runtime::runtime::Runtime;
use fvm_ipld_bitfield::BitField;
use fvm_shared::{bigint::BigInt, clock::ChainEpoch, sector::SectorNumber};

mod util;
use num_traits::Zero;
use util::*;

// an expriration ~10 days greater than effective min expiration taking into account 30 days max
// between pre and prove commit
const DEFAULT_SECTOR_EXPIRATION: ChainEpoch = 220;

#[test]
fn valid_precommits_then_aggregate_provecommit() {
    let period_offset = ChainEpoch::from(100);

    let actor = ActorHarness::new(period_offset);
    let mut rt = actor.new_runtime();
    let balance = BigInt::from(1_000_000u64) * BigInt::from(10u64.pow(17));
    rt.add_balance(balance);
    let precommit_epoch = period_offset + 1;
    rt.set_epoch(precommit_epoch);
    actor.construct_and_verify(&mut rt);
    let dl_info = actor.deadline(&rt);

    // make a good commitment for the proof to target

    let prove_commit_epoch = precommit_epoch + rt.policy.pre_commit_challenge_delay + 1;
    // something on deadline boundary but > 180 days
    let expiration =
        dl_info.period_end() + rt.policy.wpost_proving_period * DEFAULT_SECTOR_EXPIRATION;
    // fill the sector with verified seals
    let sector_weight = actor.sector_size as i64 * (expiration - prove_commit_epoch);
    let deal_weight = BigInt::zero();
    let verified_deal_weight = sector_weight;

    let mut precommits = vec![];
    let mut sector_nos_bf = BitField::new();
    for i in 0..10u64 {
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

    for sector_no in sector_nos_bf.iter() {
        assert!(!actor.has_precommit(&mut rt, sector_no));
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
    for sector_no in sector_nos_bf.iter() {
        let sector = actor.get_sector(&rt, SectorNumber::from(sector_no));
        // expect deal weights to be transferred to on chain info
        assert_eq!(deal_weight, sector.deal_weight);
        assert_eq!(BigInt::from(verified_deal_weight), sector.verified_deal_weight);

        // expect activation epoch to be current epoch
        assert_eq!(rt.epoch, sector.activation);

        // expect initial pledge of sector to be set
        assert_eq!(expected_initial_pledge, sector.initial_pledge);

        // expect sector to be assigned a deadline/partition
        let (dlidx, pidx) =
            st.find_sector(&rt.policy, rt.store(), SectorNumber::from(sector_no)).unwrap();
        // first ten sectors should be assigned to deadline 0 and partition 0
        assert_eq!(0, dlidx);
        assert_eq!(0, pidx);
    }

    let sector_power = new_power_pair(BigInt::from(actor.sector_size as i64), qa_power);
    let ten_sectors_power = new_power_pair(10 * sector_power.raw, 10 * sector_power.qa);

    let dl_idx = 0;
    let p_idx = 0;

    let (deadline, partition) = actor.get_deadline_and_partition(&rt, dl_idx, p_idx);
    assert_eq!(10, deadline.live_sectors);

    assert!(deadline.partitions_posted.is_empty());
    assert!(deadline.early_terminations.is_empty());

    let quant = st.quant_spec_for_deadline(&rt.policy, dl_idx);
    let quantized_expiration = quant.quantize_up(expiration);

    let d_queue = actor.collect_deadline_expirations(&mut rt, &deadline);
    let mut expected_queue = HashMap::new();
    expected_queue.insert(quantized_expiration, vec![p_idx]);
    assert_eq!(expected_queue, d_queue);

    assert_eq!(partition.sectors, sector_nos_bf);
    assert!(partition.faults.is_empty());
    assert!(partition.recoveries.is_empty());
    assert!(partition.terminated.is_empty());
    assert_eq!(ten_sectors_power, partition.live_power);
    assert_eq!(new_power_pair_zero(), partition.faulty_power);
    assert_eq!(new_power_pair_zero(), partition.recovering_power);

    let p_queue = actor.collect_partition_expirations(&rt, &partition);
    let entry = p_queue.get(&quantized_expiration).cloned().unwrap();
    assert_eq!(entry.on_time_sectors, sector_nos_bf);
    assert!(entry.early_sectors.is_empty());
    assert_eq!(ten_sectors_initial_pledge, entry.on_time_pledge);
    assert_eq!(ten_sectors_power, entry.active_power);
    assert_eq!(new_power_pair_zero(), entry.faulty_power);

    // expect 10x locked initial pledge of sector to be the same as pledge requirement
    assert_eq!(ten_sectors_initial_pledge, st.initial_pledge);
}
