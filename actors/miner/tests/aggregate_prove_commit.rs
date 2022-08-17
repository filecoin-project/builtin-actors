use std::collections::HashMap;

use fil_actor_market::DealSpaces;
use fil_actor_miner::{
    initial_pledge_for_power, qa_power_for_weight, PowerPair, QUALITY_BASE_MULTIPLIER,
    VERIFIED_DEAL_WEIGHT_MULTIPLIER,
};
use fil_actors_runtime::runtime::Runtime;
use fvm_ipld_bitfield::BitField;
use fvm_shared::{bigint::BigInt, clock::ChainEpoch, econ::TokenAmount};

mod util;
use fil_actors_runtime::test_utils::make_piece_cid;
use num_traits::Zero;
use util::*;

// an expiration ~10 days greater than effective min expiration taking into account 30 days max
// between pre and prove commit
const DEFAULT_SECTOR_EXPIRATION: ChainEpoch = 220;

#[test]
fn valid_precommits_then_aggregate_provecommit() {
    let period_offset = ChainEpoch::from(100);

    let actor = ActorHarness::new(period_offset);
    let mut rt = actor.new_runtime();
    rt.add_balance(BIG_BALANCE.clone());
    let precommit_epoch = period_offset + 1;
    rt.set_epoch(precommit_epoch);
    actor.construct_and_verify(&mut rt);
    let dl_info = actor.deadline(&rt);

    // make a good commitment for the proof to target

    let prove_commit_epoch = precommit_epoch + rt.policy.pre_commit_challenge_delay + 1;
    // something on deadline boundary but > 180 days
    let verified_deal_space = actor.sector_size as u64;
    let expiration =
        dl_info.period_end() + rt.policy.wpost_proving_period * DEFAULT_SECTOR_EXPIRATION;
    // fill the sector with verified seals
    let duration = expiration - prove_commit_epoch;
    let deal_spaces = DealSpaces {
        deal_space: BigInt::zero(),
        verified_deal_space: BigInt::from(verified_deal_space),
    };

    let mut precommits = vec![];
    let mut sector_nos_bf = BitField::new();
    for i in 0..10u64 {
        sector_nos_bf.set(i);
        let precommit_params =
            actor.make_pre_commit_params(i, precommit_epoch - 1, expiration, vec![1]);
        let config = PreCommitConfig::new(Some(make_piece_cid("1".as_bytes())));
        let precommit = actor.pre_commit_sector_and_get(&mut rt, precommit_params, config, i == 0);
        precommits.push(precommit);
    }

    // run prove commit logic
    rt.set_epoch(prove_commit_epoch);
    rt.set_balance(TokenAmount::from_whole(1000));

    let mut pcc = ProveCommitConfig::empty();
    for pc in &precommits {
        pcc.add_verified_deals(
            pc.info.sector_number,
            vec![test_verified_deal(verified_deal_space)],
        );
    }

    actor
        .prove_commit_aggregate_sector(
            &mut rt,
            pcc,
            precommits,
            make_prove_commit_aggregate(&sector_nos_bf),
            &TokenAmount::zero(),
        )
        .unwrap();

    // expect precommits to have been removed
    let st = actor.get_state(&rt);

    for sector_no in sector_nos_bf.iter() {
        assert!(!actor.has_precommit(&rt, sector_no));
    }

    // expect deposit to have been transferred to initial pledges
    assert!(st.pre_commit_deposits.is_zero());

    // The sector is exactly full with verified deals, so expect fully verified power.
    let expected_power = BigInt::from(actor.sector_size as i64)
        * (VERIFIED_DEAL_WEIGHT_MULTIPLIER.clone() / QUALITY_BASE_MULTIPLIER.clone());
    let deal_weight = deal_spaces.deal_space * duration;
    let verified_deal_weight = deal_spaces.verified_deal_space * duration;
    let qa_power = qa_power_for_weight(
        actor.sector_size,
        expiration - rt.epoch,
        &deal_weight,
        &verified_deal_weight,
    );
    assert_eq!(expected_power, qa_power);
    let expected_initial_pledge = initial_pledge_for_power(
        &qa_power,
        &actor.baseline_power,
        &actor.epoch_reward_smooth,
        &actor.epoch_qa_power_smooth,
        &rt.total_fil_circ_supply(),
    );
    let ten_sectors_initial_pledge = BigInt::from(10i32) * expected_initial_pledge.clone();
    assert_eq!(ten_sectors_initial_pledge, st.initial_pledge);

    // expect new onchain sector
    for sector_no in sector_nos_bf.iter() {
        let sector = actor.get_sector(&rt, sector_no);
        // expect deal weights to be transferred to on chain info
        assert_eq!(deal_weight, sector.deal_weight);
        assert_eq!(verified_deal_weight, sector.verified_deal_weight);

        // expect activation epoch to be current epoch
        assert_eq!(rt.epoch, sector.activation);

        // expect initial pledge of sector to be set
        assert_eq!(expected_initial_pledge, sector.initial_pledge);

        // expect sector to be assigned a deadline/partition
        let (dlidx, pidx) = st.find_sector(&rt.policy, rt.store(), sector_no).unwrap();
        // first ten sectors should be assigned to deadline 0 and partition 0
        assert_eq!(0, dlidx);
        assert_eq!(0, pidx);
    }

    let sector_power = PowerPair::new(BigInt::from(actor.sector_size as i64), qa_power);
    let ten_sectors_power = PowerPair::new(
        BigInt::from(10u32) * sector_power.raw,
        BigInt::from(10u32) * sector_power.qa,
    );

    let dl_idx = 0;
    let p_idx = 0;

    let (deadline, partition) = actor.get_deadline_and_partition(&rt, dl_idx, p_idx);
    assert_eq!(10, deadline.live_sectors);

    assert!(deadline.partitions_posted.is_empty());
    assert!(deadline.early_terminations.is_empty());

    let quant = st.quant_spec_for_deadline(&rt.policy, dl_idx);
    let quantized_expiration = quant.quantize_up(expiration);

    let d_queue = actor.collect_deadline_expirations(&rt, &deadline);
    let mut expected_queue = HashMap::new();
    expected_queue.insert(quantized_expiration, vec![p_idx]);
    assert_eq!(expected_queue, d_queue);

    assert_eq!(partition.sectors, sector_nos_bf);
    assert!(partition.faults.is_empty());
    assert!(partition.recoveries.is_empty());
    assert!(partition.terminated.is_empty());
    assert_eq!(ten_sectors_power, partition.live_power);
    assert_eq!(PowerPair::zero(), partition.faulty_power);
    assert_eq!(PowerPair::zero(), partition.recovering_power);

    let p_queue = actor.collect_partition_expirations(&rt, &partition);
    let entry = p_queue.get(&quantized_expiration).cloned().unwrap();
    assert_eq!(entry.on_time_sectors, sector_nos_bf);
    assert!(entry.faulty_sectors.is_empty());
    assert_eq!(ten_sectors_initial_pledge, entry.on_time_pledge);
    assert_eq!(ten_sectors_power, entry.active_power);
    assert_eq!(PowerPair::zero(), entry.faulty_power);
}
