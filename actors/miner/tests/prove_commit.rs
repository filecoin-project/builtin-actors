use fil_actor_market::{DealSpaces, SectorDealData};
use fil_actor_miner::{
    initial_pledge_for_power, max_prove_commit_duration, pre_commit_deposit_for_power,
    qa_power_for_weight, qa_power_max, PowerPair, PreCommitSectorBatchParams, VestSpec,
};
use fil_actors_runtime::test_utils::make_piece_cid;
use fil_actors_runtime::{runtime::Runtime, test_utils::expect_abort, DealWeight};
use fvm_shared::{
    bigint::{BigInt, Zero},
    clock::ChainEpoch,
    econ::TokenAmount,
    error::ExitCode,
    sector::{StoragePower, MAX_SECTOR_NUMBER},
    smooth::FilterEstimate,
};
use std::collections::HashMap;

mod util;

use util::*;

// an expiration ~10 days greater than effective min expiration taking into account 30 days max
// between pre and prove commit
const DEFAULT_SECTOR_EXPIRATION: ChainEpoch = 220;

const VERIFIED_DEAL_WEIGHT_MULTIPLIER: u64 = 100;
const QUALITY_BASE_MULTIPLIER: u64 = 10;
const PERIOD_OFFSET: ChainEpoch = 100;

#[test]
fn prove_single_sector() {
    let h = ActorHarness::new(PERIOD_OFFSET);
    let rt = h.new_runtime();
    rt.balance.replace(BIG_BALANCE.clone());

    let precommit_epoch = PERIOD_OFFSET + 1;
    rt.set_epoch(precommit_epoch);

    h.construct_and_verify(&rt);
    let dl_info = h.deadline(&rt);

    // Make a good commitment for the proof to target.
    // Use the max sector number to make sure everything works.
    let sector_no = MAX_SECTOR_NUMBER;
    let prove_commit_epoch = precommit_epoch + rt.policy.pre_commit_challenge_delay + 1;
    let expiration =
        dl_info.period_end() + DEFAULT_SECTOR_EXPIRATION * rt.policy.wpost_proving_period; // something on deadline boundary but > 180 days
                                                                                           // Fill the sector with verified deals
    let verified_deal = test_verified_deal(h.sector_size as u64);
    let deal_spaces = DealSpaces {
        deal_space: BigInt::zero(),
        verified_deal_space: BigInt::from(verified_deal.size.0),
    };

    // Pre-commit with a deal in order to exercise non-zero deal weights.
    let precommit_params =
        h.make_pre_commit_params(sector_no, precommit_epoch - 1, expiration, vec![1]);
    let precommit =
        h.pre_commit_sector_and_get(&rt, precommit_params, PreCommitConfig::empty(), true);

    let pwr_estimate = qa_power_max(h.sector_size);
    let expected_deposit = pre_commit_deposit_for_power(
        &h.epoch_reward_smooth,
        &h.epoch_qa_power_smooth,
        &pwr_estimate,
    );
    assert_eq!(expected_deposit, precommit.pre_commit_deposit);

    // expect total precommit deposit to equal our new deposit
    let st = h.get_state(&rt);
    assert_eq!(expected_deposit, st.pre_commit_deposits);

    // run prove commit logic
    rt.set_epoch(prove_commit_epoch);
    rt.balance.replace(TokenAmount::from_whole(1000));
    let mut pcc = ProveCommitConfig::empty();
    pcc.add_verified_deals(sector_no, vec![verified_deal]);

    let sector = h
        .prove_commit_sector_and_confirm(
            &rt,
            &precommit,
            h.make_prove_commit_params(sector_no),
            pcc,
        )
        .unwrap();

    assert_eq!(precommit.info.seal_proof, sector.seal_proof);
    assert_eq!(precommit.info.sealed_cid, sector.sealed_cid);
    assert_eq!(precommit.info.deal_ids, sector.deal_ids);
    assert_eq!(*rt.epoch.borrow(), sector.activation);
    assert_eq!(precommit.info.expiration, sector.expiration);

    // expect precommit to have been removed
    let st = h.get_state(&rt);
    let found = st.get_precommitted_sector(&rt.store, sector_no).unwrap();
    assert!(found.is_none());

    // expect deposit to have been transferred to initial pledges
    assert!(st.pre_commit_deposits.is_zero());

    // The sector is exactly full with verified deals, so expect fully verified power.
    let duration = precommit.info.expiration - prove_commit_epoch;
    let deal_weight = deal_spaces.deal_space * duration;
    let verified_deal_weight = deal_spaces.verified_deal_space * duration;
    let expected_power = StoragePower::from(h.sector_size as u64)
        * (VERIFIED_DEAL_WEIGHT_MULTIPLIER / QUALITY_BASE_MULTIPLIER);
    let qa_power =
        qa_power_for_weight(h.sector_size, duration, &deal_weight, &verified_deal_weight);
    assert_eq!(expected_power, qa_power);
    let sector_power =
        PowerPair { raw: StoragePower::from(h.sector_size as u64), qa: qa_power.clone() };

    // expect deal weights to be transferred to on chain info
    assert_eq!(deal_weight, sector.deal_weight);
    assert_eq!(verified_deal_weight, sector.verified_deal_weight);

    // expect initial plege of sector to be set, and be total pledge requirement
    let expected_initial_pledge = initial_pledge_for_power(
        &qa_power,
        &h.baseline_power,
        &h.epoch_reward_smooth,
        &h.epoch_qa_power_smooth,
        &rt.total_fil_circ_supply(),
    );
    assert_eq!(expected_initial_pledge, sector.initial_pledge);
    assert_eq!(expected_initial_pledge, st.initial_pledge);

    // expect sector to be assigned a deadline/partition
    let (dl_idx, p_idx) = st.find_sector(&rt.policy, rt.store(), sector_no).unwrap();
    let (deadline, partition) = h.get_deadline_and_partition(&rt, dl_idx, p_idx);
    assert_eq!(1, deadline.live_sectors);
    assert!(deadline.partitions_posted.is_empty());
    assert!(deadline.early_terminations.is_empty());

    let quant = st.quant_spec_for_deadline(&rt.policy, dl_idx);
    let quantized_expiration = quant.quantize_up(precommit.info.expiration);

    let d_queue = h.collect_deadline_expirations(&rt, &deadline);
    assert_eq!(HashMap::from([(quantized_expiration, vec![p_idx])]), d_queue);

    assert_bitfield_equals(&partition.sectors, &[sector_no]);
    assert!(partition.faults.is_empty());
    assert!(partition.recoveries.is_empty());
    assert!(partition.terminated.is_empty());
    assert_eq!(sector_power, partition.live_power);
    assert!(partition.faulty_power.is_zero());
    assert!(partition.recovering_power.is_zero());

    let p_queue = h.collect_partition_expirations(&rt, &partition);
    let entry = &p_queue[&quantized_expiration];
    assert_bitfield_equals(&entry.on_time_sectors, &[sector_no]);
    assert!(entry.early_sectors.is_empty());
    assert_eq!(expected_initial_pledge, entry.on_time_pledge);
    assert_eq!(sector_power, entry.active_power);
    assert!(entry.faulty_power.is_zero());
}

#[test]
fn prove_sectors_from_batch_pre_commit() {
    let h = ActorHarness::new(PERIOD_OFFSET);
    let rt = h.new_runtime();
    rt.balance.replace(BIG_BALANCE.clone());

    let precommit_epoch = PERIOD_OFFSET + 1;
    rt.set_epoch(precommit_epoch);

    h.construct_and_verify(&rt);
    let dl_info = h.deadline(&rt);

    let sector_expiration =
        dl_info.period_end() + DEFAULT_SECTOR_EXPIRATION * rt.policy.wpost_proving_period;

    let sectors = vec![
        h.make_pre_commit_params(100, precommit_epoch - 1, sector_expiration, vec![]),
        h.make_pre_commit_params(101, precommit_epoch - 1, sector_expiration, vec![1]), // 1 * 32GiB verified deal
        h.make_pre_commit_params(102, precommit_epoch - 1, sector_expiration, vec![2, 3]), // 2 * 16GiB verified deals
    ];

    let deal_space: i64 = 32 << 30;
    let prove_commit_epoch = precommit_epoch + rt.policy.pre_commit_challenge_delay + 1;
    let deal_lifespan = sector_expiration - prove_commit_epoch;
    let verified_deal1 = test_verified_deal(deal_space as u64);
    let verified_deal2 = test_verified_deal(deal_space as u64 / 2);
    let verified_deal3 = test_verified_deal(deal_space as u64 / 2);
    let deal_weight = DealWeight::zero();
    let verified_deal_weight = deal_space * DealWeight::from(deal_lifespan);

    let conf = PreCommitBatchConfig {
        sector_deal_data: vec![
            SectorDealData { commd: None },
            SectorDealData { commd: Some(make_piece_cid(b"1")) },
            SectorDealData { commd: Some(make_piece_cid(b"2|3")) },
        ],
        first_for_miner: true,
    };

    let precommits = h.pre_commit_sector_batch_and_get(
        &rt,
        PreCommitSectorBatchParams { sectors },
        &conf,
        &TokenAmount::zero(),
    );

    rt.set_epoch(prove_commit_epoch);

    let no_deal_power = qa_power_for_weight(
        h.sector_size,
        sector_expiration - prove_commit_epoch,
        &DealWeight::zero(),
        &DealWeight::zero(),
    );
    let no_deal_pledge = initial_pledge_for_power(
        &no_deal_power,
        &h.baseline_power,
        &h.epoch_reward_smooth,
        &h.epoch_qa_power_smooth,
        &rt.total_fil_circ_supply(),
    );
    let full_deal_power = qa_power_for_weight(
        h.sector_size,
        sector_expiration - prove_commit_epoch,
        &deal_weight,
        &verified_deal_weight,
    );
    let expected_power = StoragePower::from(h.sector_size as u64)
        * (VERIFIED_DEAL_WEIGHT_MULTIPLIER / QUALITY_BASE_MULTIPLIER);
    assert_eq!(full_deal_power, expected_power);
    let full_deal_pledge = initial_pledge_for_power(
        &full_deal_power,
        &h.baseline_power,
        &h.epoch_reward_smooth,
        &h.epoch_qa_power_smooth,
        &rt.total_fil_circ_supply(),
    );

    // Prove just the first sector, with no deals
    {
        let precommit = &precommits[0];
        let sector = h
            .prove_commit_sector_and_confirm(
                &rt,
                precommit,
                h.make_prove_commit_params(precommit.info.sector_number),
                ProveCommitConfig::default(),
            )
            .unwrap();
        assert_eq!(*rt.epoch.borrow(), sector.activation);
        let st = h.get_state(&rt);
        let expected_deposits = 2 * pre_commit_deposit_for_power(
            &h.epoch_reward_smooth,
            &h.epoch_qa_power_smooth,
            &qa_power_max(h.sector_size),
        ); // first sector deposit released

        assert_eq!(expected_deposits, st.pre_commit_deposits);

        // Expect power/pledge for a sector with no deals
        assert_eq!(no_deal_pledge, sector.initial_pledge);
        assert_eq!(no_deal_pledge, st.initial_pledge);
    }
    // Prove the next, with one deal
    {
        let precommit = &precommits[1];
        let mut pcc = ProveCommitConfig::empty();
        pcc.add_verified_deals(precommit.info.sector_number, vec![verified_deal1]);
        let sector = h
            .prove_commit_sector_and_confirm(
                &rt,
                precommit,
                h.make_prove_commit_params(precommit.info.sector_number),
                pcc,
            )
            .unwrap();
        assert_eq!(*rt.epoch.borrow(), sector.activation);
        let st = h.get_state(&rt);
        let expected_deposits = pre_commit_deposit_for_power(
            &h.epoch_reward_smooth,
            &h.epoch_qa_power_smooth,
            &qa_power_max(h.sector_size),
        ); // first and second deposit released

        assert_eq!(expected_deposits, st.pre_commit_deposits);

        // Expect power/pledge for the two sectors (only this one having any deal weight)
        assert_eq!(full_deal_pledge, sector.initial_pledge);
        assert_eq!(&no_deal_pledge + &full_deal_pledge, st.initial_pledge);
    }
    // Prove the last
    {
        let precommit = &precommits[2];
        let mut pcc = ProveCommitConfig::empty();
        pcc.add_verified_deals(precommit.info.sector_number, vec![verified_deal2, verified_deal3]);
        let sector = h
            .prove_commit_sector_and_confirm(
                &rt,
                precommit,
                h.make_prove_commit_params(precommit.info.sector_number),
                pcc,
            )
            .unwrap();
        assert_eq!(*rt.epoch.borrow(), sector.activation);
        let st = h.get_state(&rt);
        assert!(st.pre_commit_deposits.is_zero());

        // Expect power/pledge for the three sectors
        assert_eq!(&full_deal_pledge, &sector.initial_pledge);
        assert_eq!(&no_deal_pledge + &full_deal_pledge + &full_deal_pledge, st.initial_pledge);
    }
}

#[test]
fn invalid_proof_rejected() {
    let h = ActorHarness::new(PERIOD_OFFSET);
    let rt = h.new_runtime();
    rt.balance.replace(BIG_BALANCE.clone());

    let precommit_epoch = PERIOD_OFFSET + 1;
    rt.set_epoch(precommit_epoch);

    h.construct_and_verify(&rt);
    let deadline = h.deadline(&rt);

    // Make a good commitment for the proof to target.
    let sector_no = 100;
    let params = h.make_pre_commit_params(
        sector_no,
        precommit_epoch - 1,
        deadline.period_end() + DEFAULT_SECTOR_EXPIRATION * rt.policy.wpost_proving_period,
        vec![1],
    );
    let precommit = h.pre_commit_sector_and_get(&rt, params, PreCommitConfig::default(), true);

    // Sector pre-commitment missing.
    rt.set_epoch(precommit_epoch + rt.policy.pre_commit_challenge_delay + 1);
    expect_abort(
        ExitCode::USR_NOT_FOUND,
        h.prove_commit_sector_and_confirm(
            &rt,
            &precommit,
            h.make_prove_commit_params(sector_no + 1),
            ProveCommitConfig::empty(),
        ),
    );
    rt.reset();

    // Too late.
    rt.set_epoch(
        precommit_epoch
            + max_prove_commit_duration(&rt.policy, precommit.info.seal_proof).unwrap()
            + 1,
    );
    expect_abort(
        ExitCode::USR_ILLEGAL_ARGUMENT,
        h.prove_commit_sector_and_confirm(
            &rt,
            &precommit,
            h.make_prove_commit_params(sector_no),
            ProveCommitConfig::empty(),
        ),
    );
    rt.reset();

    // Too early.
    rt.set_epoch(precommit_epoch + rt.policy.pre_commit_challenge_delay - 1);
    expect_abort(
        ExitCode::USR_FORBIDDEN,
        h.prove_commit_sector_and_confirm(
            &rt,
            &precommit,
            h.make_prove_commit_params(sector_no),
            ProveCommitConfig::empty(),
        ),
    );
    rt.reset();

    // Set the right epoch for all following tests
    rt.set_epoch(precommit_epoch + rt.policy.pre_commit_challenge_delay + 1);

    // Invalid deals (market ActivateDeals aborts)
    let verify_deals_exit =
        HashMap::from([(precommit.info.sector_number, ExitCode::USR_ILLEGAL_ARGUMENT)]);
    expect_abort(
        ExitCode::USR_ILLEGAL_ARGUMENT,
        h.prove_commit_sector_and_confirm(
            &rt,
            &precommit,
            h.make_prove_commit_params(sector_no),
            ProveCommitConfig { verify_deals_exit, ..Default::default() },
        ),
    );
    rt.reset();

    rt.balance.replace(TokenAmount::from_whole(1_000));

    let prove_commit = h.make_prove_commit_params(sector_no);
    h.prove_commit_sector_and_confirm(&rt, &precommit, prove_commit, ProveCommitConfig::empty())
        .unwrap();
    let st = h.get_state(&rt);

    // Verify new sectors
    // TODO minerstate
    //newSectors, err := st.NewSectors.All(miner.SectorsMax)
    //require.NoError(t, err)
    //assert.Equal(t, []uint64{uint64(sectorNo)}, newSectors)
    // Verify pledge lock-up
    assert!(st.initial_pledge.is_positive());
    rt.reset();

    // Duplicate proof (sector no-longer pre-committed)
    expect_abort(
        ExitCode::USR_NOT_FOUND,
        h.prove_commit_sector_and_confirm(
            &rt,
            &precommit,
            h.make_prove_commit_params(sector_no),
            ProveCommitConfig::empty(),
        ),
    );
    rt.reset();
    h.check_state(&rt);
}

#[test]
fn prove_commit_aborts_if_pledge_requirement_not_met() {
    let mut h = ActorHarness::new(PERIOD_OFFSET);
    let rt = h.new_runtime();
    rt.balance.replace(BIG_BALANCE.clone());

    h.construct_and_verify(&rt);

    // Set the circulating supply high and expected reward low in order to coerce
    // pledge requirements (BR + share of money supply, but capped at 1FIL)
    // to exceed pre-commit deposit (BR only).
    rt.set_circulating_supply(TokenAmount::from_whole(100_000_000));
    h.epoch_reward_smooth = FilterEstimate::new(BigInt::from(1e15 as u64), BigInt::zero());

    // prove one sector to establish collateral and locked funds
    let sectors =
        h.commit_and_prove_sectors(&rt, 1, DEFAULT_SECTOR_EXPIRATION as u64, vec![], true);

    // precommit another sector so we may prove it
    let expiration = DEFAULT_SECTOR_EXPIRATION * rt.policy.wpost_proving_period + PERIOD_OFFSET - 1;
    let precommit_epoch = *rt.epoch.borrow() + 1;
    rt.set_epoch(precommit_epoch);
    let params =
        h.make_pre_commit_params(h.next_sector_no, *rt.epoch.borrow() - 1, expiration, vec![]);
    let precommit = h.pre_commit_sector_and_get(&rt, params, PreCommitConfig::default(), false);

    // Confirm the unlocked PCD will not cover the new IP
    assert!(sectors[0].initial_pledge > precommit.pre_commit_deposit);

    // Set balance to exactly cover locked funds.
    let st = h.get_state(&rt);
    rt.balance.replace(&st.pre_commit_deposits + &st.initial_pledge + &st.locked_funds);

    rt.set_epoch(
        precommit_epoch + max_prove_commit_duration(&rt.policy, h.seal_proof_type).unwrap() - 1,
    );
    expect_abort(
        ExitCode::USR_INSUFFICIENT_FUNDS,
        h.prove_commit_sector_and_confirm(
            &rt,
            &precommit,
            h.make_prove_commit_params(h.next_sector_no),
            ProveCommitConfig::empty(),
        ),
    );
    rt.reset();

    // succeeds with enough free balance (enough to cover 2x IP)
    rt.balance.replace(
        &st.pre_commit_deposits + &st.initial_pledge + &st.initial_pledge + &st.locked_funds,
    );
    h.prove_commit_sector_and_confirm(
        &rt,
        &precommit,
        h.make_prove_commit_params(h.next_sector_no),
        ProveCommitConfig::empty(),
    )
    .unwrap();
    h.check_state(&rt);
}

#[test]
fn drop_invalid_prove_commit_while_processing_valid_one() {
    let mut h = ActorHarness::new(PERIOD_OFFSET);
    let rt = h.new_runtime();
    rt.balance.replace(BIG_BALANCE.clone());

    h.construct_and_verify(&rt);

    // make two precommits
    let expiration = DEFAULT_SECTOR_EXPIRATION * rt.policy.wpost_proving_period + PERIOD_OFFSET - 1;
    let precommit_epoch = *rt.epoch.borrow() + 1;
    rt.set_epoch(precommit_epoch);
    let params_a =
        h.make_pre_commit_params(h.next_sector_no, *rt.epoch.borrow() - 1, expiration, vec![1]);
    let pre_commit_a = h.pre_commit_sector_and_get(&rt, params_a, PreCommitConfig::default(), true);
    let sector_no_a = h.next_sector_no;
    h.next_sector_no += 1;
    let params_b =
        h.make_pre_commit_params(h.next_sector_no, *rt.epoch.borrow() - 1, expiration, vec![2]);
    let pre_commit_b =
        h.pre_commit_sector_and_get(&rt, params_b, PreCommitConfig::default(), false);
    let sector_no_b = h.next_sector_no;

    // handle both prove commits in the same epoch
    rt.set_epoch(
        precommit_epoch + max_prove_commit_duration(&rt.policy, h.seal_proof_type).unwrap() - 1,
    );

    h.prove_commit_sector(&rt, &pre_commit_a, h.make_prove_commit_params(sector_no_a)).unwrap();
    h.prove_commit_sector(&rt, &pre_commit_b, h.make_prove_commit_params(sector_no_b)).unwrap();

    let conf = ProveCommitConfig {
        verify_deals_exit: HashMap::from([(sector_no_a, ExitCode::USR_ILLEGAL_ARGUMENT)]),
        ..Default::default()
    };
    h.confirm_sector_proofs_valid(&rt, conf, vec![pre_commit_a, pre_commit_b]).unwrap();
    let st = h.get_state(&rt);
    assert!(st.get_sector(&rt.store, sector_no_a).unwrap().is_none());
    assert!(st.get_sector(&rt.store, sector_no_b).unwrap().is_some());
    h.check_state(&rt);
}

#[test]
fn prove_commit_just_after_period_start_permits_post() {
    let h = ActorHarness::new(PERIOD_OFFSET);
    let rt = h.new_runtime();
    rt.balance.replace(BIG_BALANCE.clone());

    // Epoch PERIOD_OFFSET+1 should be at the beginning of the miner's proving period so there will be time to commit
    // and PoSt a sector.
    rt.set_epoch(PERIOD_OFFSET + 1);
    h.construct_and_verify(&rt);

    // Commit a sector the very next epoch
    rt.set_epoch(PERIOD_OFFSET + 2);
    let sector =
        h.commit_and_prove_sector(&rt, MAX_SECTOR_NUMBER, DEFAULT_SECTOR_EXPIRATION, vec![]);

    // advance cron to activate power.
    h.advance_and_submit_posts(&rt, &[sector]);
    h.check_state(&rt);
}

#[test]
fn sector_with_non_positive_lifetime_is_skipped_in_confirmation() {
    let h = ActorHarness::new(PERIOD_OFFSET);
    let rt = h.new_runtime();
    rt.balance.replace(BIG_BALANCE.clone());

    let precommit_epoch = PERIOD_OFFSET + 1;
    rt.set_epoch(precommit_epoch);

    h.construct_and_verify(&rt);
    let deadline = h.deadline(&rt);

    let sector_no = 100;
    let params = h.make_pre_commit_params(
        sector_no,
        precommit_epoch - 1,
        deadline.period_end() + DEFAULT_SECTOR_EXPIRATION * rt.policy.wpost_proving_period,
        vec![],
    );
    let precommit = h.pre_commit_sector_and_get(&rt, params, PreCommitConfig::default(), true);

    // precommit at correct epoch
    let epoch = *rt.epoch.borrow();
    rt.set_epoch(epoch + rt.policy.pre_commit_challenge_delay + 1);
    h.prove_commit_sector(&rt, &precommit, h.make_prove_commit_params(sector_no)).unwrap();

    // confirm at sector expiration (this probably can't happen)
    rt.set_epoch(precommit.info.expiration);
    // sector skipped but no failure occurs
    h.confirm_sector_proofs_valid(&rt, ProveCommitConfig::empty(), vec![precommit.clone()])
        .unwrap();

    // it still skips if sector lifetime is negative
    rt.set_epoch(precommit.info.expiration + 1);
    h.confirm_sector_proofs_valid(&rt, ProveCommitConfig::empty(), vec![precommit.clone()])
        .unwrap();

    // it fails up to the miniumum expiration
    rt.set_epoch(precommit.info.expiration - rt.policy.min_sector_expiration + 1);
    h.confirm_sector_proofs_valid(&rt, ProveCommitConfig::empty(), vec![precommit]).unwrap();
    let st = h.get_state(&rt);
    assert!(st.get_sector(&rt.store, sector_no).unwrap().is_none());
    h.check_state(&rt);
}

#[test]
fn verify_proof_does_not_vest_funds() {
    let h = ActorHarness::new(PERIOD_OFFSET);
    let rt = h.new_runtime();
    rt.balance.replace(BIG_BALANCE.clone());

    let precommit_epoch = PERIOD_OFFSET + 1;
    rt.set_epoch(precommit_epoch);

    h.construct_and_verify(&rt);
    let deadline = h.deadline(&rt);

    // Make a good commitment for the proof to target.
    let sector_no = 100;
    let params = h.make_pre_commit_params(
        sector_no,
        precommit_epoch - 1,
        deadline.period_end() + DEFAULT_SECTOR_EXPIRATION * rt.policy.wpost_proving_period,
        vec![1],
    );
    let precommit = h.pre_commit_sector_and_get(&rt, params, PreCommitConfig::default(), true);

    // add 1000 tokens that vest immediately
    let mut st = h.get_state(&rt);
    let _ = st
        .add_locked_funds(
            &rt.store,
            *rt.epoch.borrow(),
            &TokenAmount::from_atto(1000),
            &VestSpec { initial_delay: 0, vest_period: 1, step_duration: 1, quantization: 1 },
        )
        .unwrap();
    rt.replace_state(&st);

    rt.set_epoch(precommit_epoch + rt.policy.pre_commit_challenge_delay + 1);
    rt.balance.replace(TokenAmount::from_whole(1000));

    let mut prove_commit = h.make_prove_commit_params(sector_no);
    prove_commit.proof.resize(192, 0);
    // The below call expects exactly the pledge delta for the proven sector, zero for any other vesting.
    h.prove_commit_sector_and_confirm(&rt, &precommit, prove_commit, ProveCommitConfig::empty())
        .unwrap();
}
