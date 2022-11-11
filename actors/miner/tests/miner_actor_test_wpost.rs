#![allow(clippy::all)]

use fil_actor_miner as miner;
use fil_actor_miner::PowerPair;
use fil_actors_runtime::runtime::DomainSeparationTag;
use fil_actors_runtime::test_utils::*;
use fil_actors_runtime::CALLER_TYPES_SIGNABLE;
use fvm_ipld_bitfield::BitField;
use fvm_ipld_encoding::RawBytes;
use fvm_shared::clock::ChainEpoch;
use fvm_shared::econ::TokenAmount;
use fvm_shared::error::ExitCode;
use fvm_shared::randomness::Randomness;
use fvm_shared::sector::RegisteredPoStProof;
use fvm_shared::sector::RegisteredSealProof;

mod util;

use num_traits::Zero;
use util::*;

// an expiration ~10 days greater than effective min expiration taking into account 30 days max
// between pre and prove commit
const DEFAULT_SECTOR_EXPIRATION: u64 = 220;

#[test]
fn basic_post_and_dispute() {
    let period_offset = ChainEpoch::from(100);
    let precommit_epoch = ChainEpoch::from(1);

    let mut h = ActorHarness::new(period_offset);
    h.set_proof_type(RegisteredSealProof::StackedDRG2KiBV1P1);

    let mut rt = h.new_runtime();
    rt.epoch = precommit_epoch;
    rt.balance.replace(BIG_BALANCE.clone());

    h.construct_and_verify(&mut rt);

    let sectors = h.commit_and_prove_sectors(&mut rt, 1, DEFAULT_SECTOR_EXPIRATION, vec![], true);
    let sector = sectors[0].clone();
    let pwr = miner::power_for_sector(h.sector_size, &sector);

    // Skip to the right deadline
    let state = h.get_state(&rt);
    let (dlidx, pidx) = state.find_sector(&rt.policy, &rt.store, sector.sector_number).unwrap();
    let dlinfo = h.advance_to_deadline(&mut rt, dlidx);

    // Submit PoSt
    let post_partitions =
        vec![miner::PoStPartition { index: pidx, skipped: make_empty_bitfield() }];
    let post_sectors = vec![sector.clone()];
    h.submit_window_post(
        &mut rt,
        &dlinfo,
        post_partitions,
        post_sectors,
        PoStConfig::with_expected_power_delta(&pwr),
    );

    // Verify proof recorded
    let deadline = h.get_deadline(&rt, dlidx);
    let deadline_bits = [pidx];
    assert_bitfield_equals(&deadline.partitions_posted, &deadline_bits);

    let posts = amt_to_vec::<miner::WindowedPoSt>(&rt, &deadline.optimistic_post_submissions);
    assert_eq!(posts.len(), 1);
    assert_bitfield_equals(&posts[0].partitions, &deadline_bits);

    // Advance to end-of-deadline cron to verify no penalties.
    h.advance_deadline(&mut rt, CronConfig::empty());
    h.check_state(&rt);

    // Proofs should exist in snapshot.
    let deadline2 = h.get_deadline(&rt, dlidx);
    assert_eq!(
        &deadline.optimistic_post_submissions,
        &deadline2.optimistic_post_submissions_snapshot
    );

    // Try a failed dispute.
    let dispute_sectors = vec![sector];
    h.dispute_window_post(&mut rt, &dlinfo, 0, &dispute_sectors, None);

    // Now a successful dispute.
    let expected_fee = miner::pledge_penalty_for_invalid_windowpost(
        &h.epoch_reward_smooth,
        &h.epoch_qa_power_smooth,
        &pwr.qa,
    );
    let expected_result = PoStDisputeResult {
        expected_power_delta: Some(-pwr),
        expected_penalty: Some(expected_fee),
        expected_reward: Some(miner::BASE_REWARD_FOR_DISPUTED_WINDOW_POST.clone()),
        expected_pledge_delta: None,
    };
    h.dispute_window_post(&mut rt, &dlinfo, 0, &dispute_sectors, Some(expected_result));
}

#[test]
fn invalid_submissions() {
    let period_offset = ChainEpoch::from(100);
    let precommit_epoch = ChainEpoch::from(1);

    let mut h = ActorHarness::new(period_offset);
    let mut rt = h.new_runtime();
    rt.epoch = precommit_epoch;
    rt.balance.replace(BIG_BALANCE.clone());

    h.construct_and_verify(&mut rt);

    let sectors = h.commit_and_prove_sectors(&mut rt, 1, DEFAULT_SECTOR_EXPIRATION, vec![], true);
    let sector = sectors[0].clone();
    let pwr = miner::power_for_sector(h.sector_size, &sector);

    // Skip to the due deadline.
    let state = h.get_state(&rt);
    let (dlidx, pidx) = state.find_sector(&rt.policy, &rt.store, sector.sector_number).unwrap();
    let dlinfo = h.advance_to_deadline(&mut rt, dlidx);

    // Invalid deadline.
    {
        let partition = miner::PoStPartition { index: pidx, skipped: make_empty_bitfield() };
        let params = miner::SubmitWindowedPoStParams {
            deadline: rt.policy.wpost_period_deadlines,
            partitions: vec![partition],
            proofs: make_post_proofs(h.window_post_proof_type),
            chain_commit_epoch: dlinfo.challenge,
            chain_commit_rand: Randomness(TEST_RANDOMNESS_ARRAY_FROM_ONE.into()),
        };
        let result = h.submit_window_post_raw(
            &mut rt,
            &dlinfo,
            vec![sector.clone()],
            params,
            PoStConfig::empty(),
        );
        expect_abort_contains_message(ExitCode::USR_ILLEGAL_ARGUMENT, "invalid deadline", result);
        rt.reset();
    }

    // No partitions.
    // This is a weird message because we don't check this precondition explicitly.
    {
        let params = miner::SubmitWindowedPoStParams {
            deadline: dlinfo.index,
            partitions: Vec::new(),
            proofs: make_post_proofs(h.window_post_proof_type),
            chain_commit_epoch: dlinfo.challenge,
            chain_commit_rand: Randomness(TEST_RANDOMNESS_ARRAY_FROM_ONE.into()),
        };
        let result = h.submit_window_post_raw(
            &mut rt,
            &dlinfo,
            vec![sector.clone()],
            params,
            PoStConfig::empty(),
        );
        expect_abort_contains_message(
            ExitCode::USR_ILLEGAL_ARGUMENT,
            "expected proof to be smaller",
            result,
        );
        rt.reset();
    }

    // Too many partitions.
    {
        let too_many = 11;
        let partitions: Vec<_> = (0u64..too_many)
            .map(|i| miner::PoStPartition { index: pidx + i, skipped: make_empty_bitfield() })
            .collect();
        let params = miner::SubmitWindowedPoStParams {
            deadline: dlinfo.index,
            partitions,
            proofs: make_post_proofs(h.window_post_proof_type),
            chain_commit_epoch: dlinfo.challenge,
            chain_commit_rand: Randomness(TEST_RANDOMNESS_ARRAY_FROM_ONE.into()),
        };
        let result = h.submit_window_post_raw(
            &mut rt,
            &dlinfo,
            vec![sector.clone()],
            params,
            PoStConfig::empty(),
        );
        expect_abort_contains_message(
            ExitCode::USR_ILLEGAL_ARGUMENT,
            "too many partitions",
            result,
        );
        rt.reset();
    }

    // Invalid partition index.
    {
        let partition = miner::PoStPartition { index: pidx + 1, skipped: make_empty_bitfield() };
        let params = miner::SubmitWindowedPoStParams {
            deadline: dlinfo.index,
            partitions: vec![partition],
            proofs: make_post_proofs(h.window_post_proof_type),
            chain_commit_epoch: dlinfo.challenge,
            chain_commit_rand: Randomness(TEST_RANDOMNESS_ARRAY_FROM_ONE.into()),
        };
        let result = h.submit_window_post_raw(
            &mut rt,
            &dlinfo,
            vec![sector.clone()],
            params,
            PoStConfig::empty(),
        );
        expect_abort_contains_message(ExitCode::USR_NOT_FOUND, "no such partition", result);
        rt.reset();
    }

    // Skip sectors that don't exist.
    {
        let partition = miner::PoStPartition { index: pidx, skipped: make_bitfield(&[123]) };
        let params = miner::SubmitWindowedPoStParams {
            deadline: dlinfo.index,
            partitions: vec![partition],
            proofs: make_post_proofs(h.window_post_proof_type),
            chain_commit_epoch: dlinfo.challenge,
            chain_commit_rand: Randomness(TEST_RANDOMNESS_ARRAY_FROM_ONE.into()),
        };
        let result = h.submit_window_post_raw(
            &mut rt,
            &dlinfo,
            vec![sector.clone()],
            params,
            PoStConfig::empty(),
        );
        expect_abort_contains_message(
            ExitCode::USR_ILLEGAL_ARGUMENT,
            "skipped faults contains sectors outside partition",
            result,
        );
        rt.reset();
    }

    // Empty proofs array.
    {
        let partition = miner::PoStPartition { index: pidx, skipped: make_empty_bitfield() };
        let params = miner::SubmitWindowedPoStParams {
            deadline: dlinfo.index,
            partitions: vec![partition],
            proofs: Vec::new(),
            chain_commit_epoch: dlinfo.challenge,
            chain_commit_rand: Randomness(TEST_RANDOMNESS_ARRAY_FROM_ONE.into()),
        };
        let result = h.submit_window_post_raw(
            &mut rt,
            &dlinfo,
            vec![sector.clone()],
            params,
            PoStConfig::empty(),
        );
        expect_abort_contains_message(
            ExitCode::USR_ILLEGAL_ARGUMENT,
            "expected exactly one proof",
            result,
        );
        rt.reset();
    }

    // Invalid proof type
    {
        let partition = miner::PoStPartition { index: pidx, skipped: make_empty_bitfield() };
        let params = miner::SubmitWindowedPoStParams {
            deadline: dlinfo.index,
            partitions: vec![partition],
            proofs: make_post_proofs(RegisteredPoStProof::StackedDRGWindow8MiBV1),
            chain_commit_epoch: dlinfo.challenge,
            chain_commit_rand: Randomness(TEST_RANDOMNESS_ARRAY_FROM_ONE.into()),
        };
        let result = h.submit_window_post_raw(
            &mut rt,
            &dlinfo,
            vec![sector.clone()],
            params,
            PoStConfig::empty(),
        );
        expect_abort_contains_message(
            ExitCode::USR_ILLEGAL_ARGUMENT,
            "proof type StackedDRGWindow8MiBV1 not allowed",
            result,
        );
        rt.reset();
    }

    // Unexpected proof type
    {
        let partition = miner::PoStPartition { index: pidx, skipped: make_empty_bitfield() };
        let params = miner::SubmitWindowedPoStParams {
            deadline: dlinfo.index,
            partitions: vec![partition],
            proofs: make_post_proofs(RegisteredPoStProof::StackedDRGWindow64GiBV1),
            chain_commit_epoch: dlinfo.challenge,
            chain_commit_rand: Randomness(TEST_RANDOMNESS_ARRAY_FROM_ONE.into()),
        };
        let result = h.submit_window_post_raw(
            &mut rt,
            &dlinfo,
            vec![sector.clone()],
            params,
            PoStConfig::empty(),
        );
        expect_abort_contains_message(
            ExitCode::USR_ILLEGAL_ARGUMENT,
            "expected proof of type",
            result,
        );
        rt.reset();
    }

    // Proof too large
    {
        let mut proofs = make_post_proofs(h.window_post_proof_type);
        proofs[0].proof_bytes = Vec::from([0u8; 192 + 1]);
        let partition = miner::PoStPartition { index: pidx, skipped: make_empty_bitfield() };
        let params = miner::SubmitWindowedPoStParams {
            deadline: dlinfo.index,
            partitions: vec![partition],
            proofs,
            chain_commit_epoch: dlinfo.challenge,
            chain_commit_rand: Randomness(TEST_RANDOMNESS_ARRAY_FROM_ONE.into()),
        };
        let result = h.submit_window_post_raw(
            &mut rt,
            &dlinfo,
            vec![sector.clone()],
            params,
            PoStConfig::empty(),
        );
        expect_abort_contains_message(
            ExitCode::USR_ILLEGAL_ARGUMENT,
            "expected proof to be smaller",
            result,
        );
        rt.reset();
    }

    // Invalid randomness type
    {
        let partition = miner::PoStPartition { index: pidx, skipped: make_empty_bitfield() };
        let params = miner::SubmitWindowedPoStParams {
            deadline: dlinfo.index,
            partitions: vec![partition],
            proofs: make_post_proofs(h.window_post_proof_type),
            chain_commit_epoch: dlinfo.challenge,
            chain_commit_rand: Randomness(b"123456789012345678901234567890123".to_vec()),
        };
        let result = h.submit_window_post_raw(
            &mut rt,
            &dlinfo,
            vec![sector.clone()],
            params,
            PoStConfig::empty(),
        );
        expect_abort_contains_message(
            ExitCode::USR_ILLEGAL_ARGUMENT,
            "bytes of randomness",
            result,
        );
        rt.reset();
    }

    // Deadline not open.
    {
        rt.epoch += rt.policy.wpost_challenge_window;
        let partition = miner::PoStPartition { index: pidx, skipped: make_empty_bitfield() };
        let params = miner::SubmitWindowedPoStParams {
            deadline: dlinfo.index,
            partitions: vec![partition],
            proofs: make_post_proofs(h.window_post_proof_type),
            chain_commit_epoch: dlinfo.challenge + rt.policy.wpost_proving_period / 2,
            chain_commit_rand: Randomness(TEST_RANDOMNESS_ARRAY_FROM_ONE.into()),
        };
        let result = h.submit_window_post_raw(
            &mut rt,
            &dlinfo,
            vec![sector.clone()],
            params,
            PoStConfig::empty(),
        );
        //      There is discrepancy with the original test, which expects deadline 2 while the error
        //      reports deadline 0.  This is because the specs-actors tests use a fixed hasher for rt.hashfunc
        //      giving a starting deadline index of 46 whereas these tests use blake2b directly
        //      giving a starting deadline of 20.  Because committing a sector takes 2 deadlines the
        //      specs-actors test does sector assignment in an immutable deadline 0 forcing assignment to
        //      deadline 2.
        expect_abort_contains_message(ExitCode::USR_ILLEGAL_ARGUMENT, "invalid deadline", result);
        rt.epoch = dlinfo.current_epoch;
        rt.reset();
    }

    // Chain commit epoch too old.
    {
        let partition = miner::PoStPartition { index: pidx, skipped: make_empty_bitfield() };
        let params = miner::SubmitWindowedPoStParams {
            deadline: dlinfo.index,
            partitions: vec![partition],
            proofs: make_post_proofs(h.window_post_proof_type),
            chain_commit_epoch: dlinfo.challenge - 1,
            chain_commit_rand: Randomness(TEST_RANDOMNESS_ARRAY_FROM_ONE.into()),
        };
        let result = h.submit_window_post_raw(
            &mut rt,
            &dlinfo,
            vec![sector.clone()],
            params,
            PoStConfig::empty(),
        );
        expect_abort_contains_message(
            ExitCode::USR_ILLEGAL_ARGUMENT,
            "expected chain commit epoch",
            result,
        );
        rt.reset();
    }

    // Chain commit epoch too new.
    {
        let partition = miner::PoStPartition { index: pidx, skipped: make_empty_bitfield() };
        let params = miner::SubmitWindowedPoStParams {
            deadline: dlinfo.index,
            partitions: vec![partition],
            proofs: make_post_proofs(h.window_post_proof_type),
            chain_commit_epoch: rt.epoch,
            chain_commit_rand: Randomness(TEST_RANDOMNESS_ARRAY_FROM_ONE.into()),
        };
        let result = h.submit_window_post_raw(
            &mut rt,
            &dlinfo,
            vec![sector.clone()],
            params,
            PoStConfig::empty(),
        );
        expect_abort_contains_message(
            ExitCode::USR_ILLEGAL_ARGUMENT,
            "must be less than the current epoch",
            result,
        );
        rt.reset();
    }

    // Mismatched randomness
    {
        let partition = miner::PoStPartition { index: pidx, skipped: make_empty_bitfield() };
        let params = miner::SubmitWindowedPoStParams {
            deadline: dlinfo.index,
            partitions: vec![partition],
            proofs: make_post_proofs(h.window_post_proof_type),
            chain_commit_epoch: dlinfo.challenge,
            chain_commit_rand: Randomness(TEST_RANDOMNESS_ARRAY_FROM_ONE.into()),
        };
        let result = h.submit_window_post_raw(
            &mut rt,
            &dlinfo,
            vec![sector.clone()],
            params,
            PoStConfig::with_randomness(TEST_RANDOMNESS_ARRAY_FROM_TWO.into()),
        );
        expect_abort_contains_message(
            ExitCode::USR_ILLEGAL_ARGUMENT,
            "randomness mismatched",
            result,
        );
        rt.reset();
    }

    // Demonstrate the good params are good.
    {
        let partition = miner::PoStPartition { index: pidx, skipped: make_empty_bitfield() };
        let params = miner::SubmitWindowedPoStParams {
            deadline: dlinfo.index,
            partitions: vec![partition],
            proofs: make_post_proofs(h.window_post_proof_type),
            chain_commit_epoch: dlinfo.challenge,
            chain_commit_rand: Randomness(TEST_RANDOMNESS_ARRAY_FROM_ONE.into()),
        };
        h.submit_window_post_raw(
            &mut rt,
            &dlinfo,
            vec![sector],
            params,
            PoStConfig::with_expected_power_delta(&pwr),
        )
        .unwrap();
        rt.verify();
    }
}

#[test]
fn duplicate_proof_rejected() {
    let period_offset = ChainEpoch::from(100);
    let precommit_epoch = ChainEpoch::from(1);

    let mut h = ActorHarness::new(period_offset);
    h.set_proof_type(RegisteredSealProof::StackedDRG2KiBV1P1);

    let mut rt = h.new_runtime();
    rt.epoch = precommit_epoch;
    rt.balance.replace(BIG_BALANCE.clone());

    h.construct_and_verify(&mut rt);

    let sectors = h.commit_and_prove_sectors(&mut rt, 1, DEFAULT_SECTOR_EXPIRATION, vec![], true);
    let sector = sectors[0].clone();
    let pwr = miner::power_for_sector(h.sector_size, &sector);

    // Skip to the due deadline.
    let state = h.get_state(&rt);
    let (dlidx, pidx) = state.find_sector(&rt.policy, &rt.store, sector.sector_number).unwrap();
    let dlinfo = h.advance_to_deadline(&mut rt, dlidx);

    // Submit PoSt
    let post_partitions =
        vec![miner::PoStPartition { index: pidx, skipped: make_empty_bitfield() }];
    let post_sectors = vec![sector.clone()];
    h.submit_window_post(
        &mut rt,
        &dlinfo,
        post_partitions,
        post_sectors,
        PoStConfig::with_expected_power_delta(&pwr),
    );

    // Verify proof recorded
    let deadline = h.get_deadline(&rt, dlidx);
    let deadline_bits = [pidx];
    assert_bitfield_equals(&deadline.partitions_posted, &deadline_bits);

    // Submit a duplicate proof for the same partition. This will be rejected because after ignoring the
    // already-proven partition, there are no sectors remaining.
    // The skipped fault declared here has no effect.
    let commit_rand = [
        1u8, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21, 22, 23, 24,
        25, 26, 27, 28, 29, 30, 31, 32,
    ];
    let partition =
        miner::PoStPartition { index: pidx, skipped: make_bitfield(&[sector.sector_number]) };
    let params = miner::SubmitWindowedPoStParams {
        deadline: dlidx,
        partitions: vec![partition],
        proofs: make_post_proofs(h.window_post_proof_type),
        chain_commit_epoch: dlinfo.challenge,
        chain_commit_rand: Randomness(commit_rand.clone().into()),
    };

    h.expect_query_network_info(&mut rt);
    rt.set_caller(*ACCOUNT_ACTOR_CODE_ID, h.worker);

    // From version 7, a duplicate is explicitly rejected.
    rt.expect_validate_caller_addr(h.caller_addrs());
    rt.expect_get_randomness_from_tickets(
        DomainSeparationTag::PoStChainCommit,
        dlinfo.challenge,
        Vec::new(),
        commit_rand,
    );

    let result = rt.call::<miner::Actor>(
        miner::Method::SubmitWindowedPoSt as u64,
        &RawBytes::serialize(params).unwrap(),
    );
    expect_abort_contains_message(
        ExitCode::USR_ILLEGAL_ARGUMENT,
        "partition already proven",
        result,
    );
    rt.reset();

    // Advance to end-of-deadline cron to verify no penalties.
    h.advance_deadline(&mut rt, CronConfig::empty());
    h.check_state(&rt);
}

#[test]
fn duplicate_proof_rejected_with_many_partitions() {
    let period_offset = ChainEpoch::from(100);
    let precommit_epoch = ChainEpoch::from(1);

    let mut h = ActorHarness::new(period_offset);
    h.set_proof_type(RegisteredSealProof::StackedDRG2KiBV1P1);

    let mut rt = h.new_runtime();
    rt.epoch = precommit_epoch;
    rt.balance.replace(BIG_BALANCE.clone());

    h.construct_and_verify(&mut rt);

    // Commit more sectors than fit in one partition in every eligible deadline, overflowing to a second partition.
    let sectors_to_commit = (rt.policy.wpost_period_deadlines - 2) * h.partition_size + 1;
    let sectors = h.commit_and_prove_sectors(
        &mut rt,
        sectors_to_commit as usize,
        DEFAULT_SECTOR_EXPIRATION,
        vec![],
        true,
    );
    let last_sector = sectors.last().unwrap();

    // Skip to the due deadline.
    let state = h.get_state(&rt);
    let (dlidx, _) = state.find_sector(&rt.policy, &rt.store, last_sector.sector_number).unwrap();
    let dlinfo = h.advance_to_deadline(&mut rt, dlidx);

    {
        // Submit PoSt for partition 0 on its own.
        let post_partitions =
            vec![miner::PoStPartition { index: 0, skipped: make_empty_bitfield() }];
        let sectors_to_prove: Vec<_> =
            (0..h.partition_size).map(|i| sectors[i as usize].clone()).collect();
        let pwr = miner::power_for_sectors(h.sector_size, &sectors_to_prove);
        h.submit_window_post(
            &mut rt,
            &dlinfo,
            post_partitions,
            sectors_to_prove,
            PoStConfig::with_expected_power_delta(&pwr),
        );
        // Verify proof recorded
        let deadline = h.get_deadline(&rt, dlidx);
        let deadline_bits = [0];
        assert_bitfield_equals(&deadline.partitions_posted, &deadline_bits);
    }
    {
        // Attempt PoSt for both partitions, thus duplicating proof for partition 0, so rejected
        let post_partitions = vec![
            miner::PoStPartition { index: 0, skipped: make_empty_bitfield() },
            miner::PoStPartition { index: 1, skipped: make_empty_bitfield() },
        ];
        let mut sectors_to_prove: Vec<_> =
            (0..h.partition_size).map(|i| sectors[i as usize].clone()).collect();
        sectors_to_prove.push(last_sector.clone());
        let pwr = miner::power_for_sectors(h.sector_size, &sectors_to_prove);

        let params = miner::SubmitWindowedPoStParams {
            deadline: dlinfo.index,
            partitions: post_partitions,
            proofs: make_post_proofs(h.window_post_proof_type),
            chain_commit_epoch: dlinfo.challenge,
            chain_commit_rand: Randomness(TEST_RANDOMNESS_ARRAY_FROM_ONE.into()),
        };
        let result = h.submit_window_post_raw(
            &mut rt,
            &dlinfo,
            sectors_to_prove,
            params,
            PoStConfig::with_expected_power_delta(&pwr),
        );
        expect_abort_contains_message(
            ExitCode::USR_ILLEGAL_ARGUMENT,
            "partition already proven",
            result,
        );
        rt.reset();
    }
    {
        // Submit PoSt for partition 1 on its own is ok.
        let post_partitions =
            vec![miner::PoStPartition { index: 1, skipped: make_empty_bitfield() }];
        let sectors_to_prove = vec![last_sector.clone()];
        let pwr = miner::power_for_sectors(h.sector_size, &sectors_to_prove);
        h.submit_window_post(
            &mut rt,
            &dlinfo,
            post_partitions,
            sectors_to_prove,
            PoStConfig::with_expected_power_delta(&pwr),
        );
        // Verify both proofs now recorded
        let deadline = h.get_deadline(&rt, dlidx);
        let deadline_bits = [0, 1];
        assert_bitfield_equals(&deadline.partitions_posted, &deadline_bits);
    }

    // Advance to end-of-deadline cron to verify no penalties.
    h.advance_deadline(&mut rt, CronConfig::empty());
    h.check_state(&rt);
}

#[test]
fn successful_recoveries_recover_power() {
    let period_offset = ChainEpoch::from(100);
    let precommit_epoch = ChainEpoch::from(1);

    let mut h = ActorHarness::new(period_offset);
    h.set_proof_type(RegisteredSealProof::StackedDRG2KiBV1P1);

    let mut rt = h.new_runtime();
    rt.epoch = precommit_epoch;
    rt.balance.replace(BIG_BALANCE.clone());

    h.construct_and_verify(&mut rt);

    let infos = h.commit_and_prove_sectors(&mut rt, 1, DEFAULT_SECTOR_EXPIRATION, vec![], true);
    let pwr = miner::power_for_sectors(h.sector_size, &infos);

    h.apply_rewards(&mut rt, BIG_REWARDS.clone(), TokenAmount::zero());
    let initial_locked = h.get_locked_funds(&rt);

    // Submit first PoSt to ensure we are sufficiently early to add a fault
    // advance to next proving period
    h.advance_and_submit_posts(&mut rt, &infos);

    // advance deadline and declare fault
    h.advance_deadline(&mut rt, CronConfig::empty());
    h.declare_faults(&mut rt, &infos);

    // advance a deadline and declare recovery
    h.advance_deadline(&mut rt, CronConfig::empty());

    // declare recovery
    let state = h.get_state(&rt);
    let (dlidx, pidx) = state.find_sector(&rt.policy, &rt.store, infos[0].sector_number).unwrap();
    let mut bf = BitField::new();
    bf.set(infos[0].sector_number);
    h.declare_recoveries(&mut rt, dlidx, pidx, bf, TokenAmount::zero()).unwrap();

    // advance to epoch when submitPoSt is due
    let mut dlinfo = h.deadline(&rt);
    while dlinfo.index != dlidx {
        dlinfo = h.advance_deadline(&mut rt, CronConfig::empty());
    }

    // Now submit PoSt
    // Power should return for recovered sector.
    let cfg = PoStConfig::with_expected_power_delta(&pwr);
    let partition = miner::PoStPartition { index: pidx, skipped: make_empty_bitfield() };
    h.submit_window_post(&mut rt, &dlinfo, vec![partition], infos.clone(), cfg);

    // faulty power has been removed, partition no longer has faults or recoveries
    let (deadline, partition) = h.find_sector(&rt, infos[0].sector_number);
    assert_eq!(miner::PowerPair::zero(), deadline.faulty_power);
    assert_eq!(miner::PowerPair::zero(), partition.faulty_power);
    assert!(partition.faults.is_empty());
    assert!(partition.recoveries.is_empty());

    // We restored power, so we should not have recorded a post.
    let deadline = h.get_deadline(&rt, dlidx);
    assert_bitfield_equals(&deadline.partitions_posted, &[pidx]);

    let posts = amt_to_vec::<miner::WindowedPoSt>(&rt, &deadline.optimistic_post_submissions);
    assert!(posts.is_empty());

    // Next deadline cron does not charge for the fault
    h.advance_deadline(&mut rt, CronConfig::empty());

    assert_eq!(initial_locked, h.get_locked_funds(&rt));

    h.check_state(&rt);
}

#[test]
fn skipped_faults_adjust_power() {
    let period_offset = ChainEpoch::from(100);
    let precommit_epoch = ChainEpoch::from(1);

    let mut h = ActorHarness::new(period_offset);
    h.set_proof_type(RegisteredSealProof::StackedDRG2KiBV1P1);

    let mut rt = h.new_runtime();
    rt.epoch = precommit_epoch;
    rt.balance.replace(BIG_BALANCE.clone());

    h.construct_and_verify(&mut rt);

    let infos = h.commit_and_prove_sectors(&mut rt, 2, DEFAULT_SECTOR_EXPIRATION, vec![], true);

    h.apply_rewards(&mut rt, BIG_REWARDS.clone(), TokenAmount::zero());

    // Skip to the due deadline.
    let state = h.get_state(&rt);
    let (dlidx, pidx) = state.find_sector(&rt.policy, &rt.store, infos[0].sector_number).unwrap();
    let (dlidx2, pidx2) = state.find_sector(&rt.policy, &rt.store, infos[1].sector_number).unwrap();
    assert_eq!(dlidx, dlidx2);

    let mut dlinfo = h.advance_to_deadline(&mut rt, dlidx);

    // Now submit PoSt with a skipped fault for first sector
    // First sector's power should not be activated.
    let infos1 = vec![infos[0].clone()];
    let infos2 = vec![infos[1].clone()];
    let power_active = miner::power_for_sectors(h.sector_size, &infos2);
    let partition =
        miner::PoStPartition { index: pidx, skipped: make_bitfield(&[infos1[0].sector_number]) };
    h.submit_window_post(
        &mut rt,
        &dlinfo,
        vec![partition],
        infos2.clone(),
        PoStConfig::with_expected_power_delta(&power_active),
    );

    // expect continued fault fee to be charged during cron
    let fault_fee = h.continued_fault_penalty(&infos1);
    dlinfo = h.advance_deadline(&mut rt, CronConfig::with_continued_faults_penalty(fault_fee));

    // advance to next proving period, expect no fees
    while dlinfo.index != dlidx {
        dlinfo = h.advance_deadline(&mut rt, CronConfig::empty());
    }

    // Attempt to skip second sector
    let pwr_delta = -miner::power_for_sectors(h.sector_size, &infos2);
    let partition =
        miner::PoStPartition { index: pidx2, skipped: make_bitfield(&[infos[1].sector_number]) };
    let params = miner::SubmitWindowedPoStParams {
        deadline: dlinfo.index,
        partitions: vec![partition],
        proofs: make_post_proofs(h.window_post_proof_type),
        chain_commit_epoch: dlinfo.challenge,
        chain_commit_rand: Randomness(TEST_RANDOMNESS_ARRAY_FROM_ONE.into()),
    };

    // Now all sectors are faulty so there's nothing to prove.
    let result = h.submit_window_post_raw(
        &mut rt,
        &dlinfo,
        infos2.clone(),
        params,
        PoStConfig::with_expected_power_delta(&pwr_delta),
    );
    expect_abort_contains_message(ExitCode::USR_ILLEGAL_ARGUMENT, "no active sectors", result);
    rt.reset();

    // The second sector is detected faulty but pays nothing yet.
    // Expect ongoing fault penalty for only the first, continuing-faulty sector.
    let pwr_delta = -miner::power_for_sectors(h.sector_size, &infos2);
    let fault_fee = h.continued_fault_penalty(&infos1);
    h.advance_deadline(
        &mut rt,
        CronConfig::with_detected_faults_power_delta_and_continued_faults_penalty(
            &pwr_delta, fault_fee,
        ),
    );

    h.check_state(&rt);
}

#[test]
fn skipping_all_sectors_in_a_partition_rejected() {
    let period_offset = ChainEpoch::from(100);
    let precommit_epoch = ChainEpoch::from(1);

    let mut h = ActorHarness::new(period_offset);
    h.set_proof_type(RegisteredSealProof::StackedDRG2KiBV1P1);

    let mut rt = h.new_runtime();
    rt.epoch = precommit_epoch;
    rt.balance.replace(BIG_BALANCE.clone());

    h.construct_and_verify(&mut rt);

    let infos = h.commit_and_prove_sectors(&mut rt, 2, DEFAULT_SECTOR_EXPIRATION, vec![], true);

    h.apply_rewards(&mut rt, BIG_REWARDS.clone(), TokenAmount::zero());

    // Skip to the due deadline.
    let state = h.get_state(&rt);
    let (dlidx, pidx) = state.find_sector(&rt.policy, &rt.store, infos[0].sector_number).unwrap();
    let (dlidx2, pidx2) = state.find_sector(&rt.policy, &rt.store, infos[1].sector_number).unwrap();
    assert_eq!(dlidx, dlidx2);
    assert_eq!(pidx, pidx2);

    let dlinfo = h.advance_to_deadline(&mut rt, dlidx);

    // PoSt with all sectors skipped fails to validate, leaving power un-activated.
    let partition = miner::PoStPartition {
        index: pidx,
        skipped: make_bitfield(&[infos[0].sector_number, infos[1].sector_number]),
    };
    let params = miner::SubmitWindowedPoStParams {
        deadline: dlinfo.index,
        partitions: vec![partition],
        proofs: make_post_proofs(h.window_post_proof_type),
        chain_commit_epoch: dlinfo.challenge,
        chain_commit_rand: Randomness(TEST_RANDOMNESS_ARRAY_FROM_ONE.into()),
    };
    let result =
        h.submit_window_post_raw(&mut rt, &dlinfo, infos.clone(), params, PoStConfig::empty());
    expect_abort(ExitCode::USR_ILLEGAL_ARGUMENT, result);
    rt.reset();

    // These sectors are detected faulty and pay no penalty this time.
    h.advance_deadline(&mut rt, CronConfig::with_continued_faults_penalty(TokenAmount::zero()));
    h.check_state(&rt);
}

#[test]
fn skipped_recoveries_are_penalized_and_do_not_recover_power() {
    let period_offset = ChainEpoch::from(100);
    let precommit_epoch = ChainEpoch::from(1);

    let mut h = ActorHarness::new(period_offset);
    h.set_proof_type(RegisteredSealProof::StackedDRG2KiBV1P1);

    let mut rt = h.new_runtime();
    rt.epoch = precommit_epoch;
    rt.balance.replace(BIG_BALANCE.clone());

    h.construct_and_verify(&mut rt);

    let infos = h.commit_and_prove_sectors(&mut rt, 2, DEFAULT_SECTOR_EXPIRATION, vec![], true);

    h.apply_rewards(&mut rt, BIG_REWARDS.clone(), TokenAmount::zero());

    // Submit first PoSt to ensure we are sufficiently early to add a fault
    // advance to next proving period
    h.advance_and_submit_posts(&mut rt, &infos);

    // advance deadline and declare fault on the first sector
    let infos1 = vec![infos[0].clone()];
    h.advance_deadline(&mut rt, CronConfig::empty());
    h.declare_faults(&mut rt, &infos1);

    // advance a deadline and declare recovery
    h.advance_deadline(&mut rt, CronConfig::empty());

    // declare recovery
    let state = h.get_state(&rt);
    let (dlidx, pidx) = state.find_sector(&rt.policy, &rt.store, infos[0].sector_number).unwrap();
    let mut bf = BitField::new();
    bf.set(infos[0].sector_number);
    h.declare_recoveries(&mut rt, dlidx, pidx, bf, TokenAmount::zero()).unwrap();

    // Skip to the due deadline.
    let dlinfo = h.advance_to_deadline(&mut rt, dlidx);

    // Now submit PoSt and skip recovered sector.
    // No power should be returned
    let partition =
        miner::PoStPartition { index: pidx, skipped: make_bitfield(&[infos[0].sector_number]) };
    h.submit_window_post(&mut rt, &dlinfo, vec![partition], infos.clone(), PoStConfig::empty());

    // sector will be charged ongoing fee at proving period cron
    let ongoing_fee = h.continued_fault_penalty(&infos1);
    h.advance_deadline(&mut rt, CronConfig::with_continued_faults_penalty(ongoing_fee));

    h.check_state(&rt);
}

#[test]
fn skipping_a_fault_from_the_wrong_partition_is_an_error() {
    let period_offset = ChainEpoch::from(100);
    let precommit_epoch = ChainEpoch::from(1);

    let mut h = ActorHarness::new(period_offset);
    h.set_proof_type(RegisteredSealProof::StackedDRG2KiBV1P1);

    let mut rt = h.new_runtime();
    rt.epoch = precommit_epoch;
    rt.balance.replace(BIG_BALANCE.clone());

    h.construct_and_verify(&mut rt);

    // create enough sectors that one will be in a different partition
    // TODO: remove magic number and derive from seal proof based parameter
    const N: usize = 95;
    let infos = h.commit_and_prove_sectors(&mut rt, N, DEFAULT_SECTOR_EXPIRATION, vec![], true);

    // Skip to the due deadline.
    let state = h.get_state(&rt);
    let (dlidx0, pidx0) = state.find_sector(&rt.policy, &rt.store, infos[0].sector_number).unwrap();
    let (dlidx1, pidx1) =
        state.find_sector(&rt.policy, &rt.store, infos[N - 1].sector_number).unwrap();
    let dlinfo = h.advance_to_deadline(&mut rt, dlidx0);

    // if these assertions no longer hold, the test must be changed
    assert!(dlidx0 < dlidx1);
    assert!(pidx0 != pidx1);

    // Now submit PoSt for partition 1 and skip sector from other partition
    let partition = miner::PoStPartition {
        index: pidx0,
        skipped: make_bitfield(&[infos[N - 1].sector_number]),
    };
    let params = miner::SubmitWindowedPoStParams {
        deadline: dlinfo.index,
        partitions: vec![partition],
        proofs: make_post_proofs(h.window_post_proof_type),
        chain_commit_epoch: dlinfo.challenge,
        chain_commit_rand: Randomness(TEST_RANDOMNESS_ARRAY_FROM_ONE.into()),
    };
    let result = h.submit_window_post_raw(&mut rt, &dlinfo, infos, params, PoStConfig::empty());
    expect_abort_contains_message(
        ExitCode::USR_ILLEGAL_ARGUMENT,
        "skipped faults contains sectors outside partition",
        result,
    );

    h.check_state(&rt);
}

#[test]
fn cannot_dispute_posts_when_the_challenge_window_is_open() {
    let period_offset = ChainEpoch::from(100);
    let precommit_epoch = ChainEpoch::from(1);

    let mut h = ActorHarness::new(period_offset);
    h.set_proof_type(RegisteredSealProof::StackedDRG2KiBV1P1);

    let mut rt = h.new_runtime();
    rt.epoch = precommit_epoch;
    rt.balance.replace(BIG_BALANCE.clone());

    h.construct_and_verify(&mut rt);

    let infos = h.commit_and_prove_sectors(&mut rt, 1, DEFAULT_SECTOR_EXPIRATION, vec![], true);
    let sector = infos[0].clone();
    let pwr = miner::power_for_sector(h.sector_size, &sector);

    // Skip to the due deadline.
    let state = h.get_state(&rt);
    let (dlidx, pidx) = state.find_sector(&rt.policy, &rt.store, sector.sector_number).unwrap();
    let dlinfo = h.advance_to_deadline(&mut rt, dlidx);

    // Submit PoSt
    let partition = miner::PoStPartition { index: pidx, skipped: make_empty_bitfield() };
    h.submit_window_post(
        &mut rt,
        &dlinfo,
        vec![partition],
        infos,
        PoStConfig::with_expected_power_delta(&pwr),
    );

    // Dispute it.
    let params = miner::DisputeWindowedPoStParams { deadline: dlinfo.index, post_index: 0 };

    rt.set_caller(*ACCOUNT_ACTOR_CODE_ID, h.worker);
    rt.expect_validate_caller_type((*CALLER_TYPES_SIGNABLE).to_vec());
    h.expect_query_network_info(&mut rt);

    let result = rt.call::<miner::Actor>(
        miner::Method::DisputeWindowedPoSt as u64,
        &RawBytes::serialize(params).unwrap(),
    );
    expect_abort_contains_message(
        ExitCode::USR_FORBIDDEN,
        "can only dispute window posts during the dispute window",
        result,
    );
    rt.verify();
}

#[test]
fn can_dispute_up_till_window_end_but_not_after() {
    let period_offset = ChainEpoch::from(100);
    let precommit_epoch = ChainEpoch::from(1);

    let mut h = ActorHarness::new(period_offset);
    h.set_proof_type(RegisteredSealProof::StackedDRG2KiBV1P1);

    let mut rt = h.new_runtime();
    rt.epoch = precommit_epoch;
    rt.balance.replace(BIG_BALANCE.clone());

    h.construct_and_verify(&mut rt);

    let infos = h.commit_and_prove_sectors(&mut rt, 1, DEFAULT_SECTOR_EXPIRATION, vec![], true);
    let sector = infos[0].clone();

    let state = h.get_state(&rt);
    let (dlidx, _) = state.find_sector(&rt.policy, &rt.store, sector.sector_number).unwrap();

    let nextdl = miner::DeadlineInfo::new(
        state.proving_period_start,
        dlidx,
        rt.epoch,
        rt.policy.wpost_period_deadlines,
        rt.policy.wpost_proving_period,
        rt.policy.wpost_challenge_window,
        rt.policy.wpost_challenge_lookback,
        rt.policy.fault_declaration_cutoff,
    )
    .next_not_elapsed();

    h.advance_and_submit_posts(&mut rt, &infos);
    let window_end = nextdl.close + rt.policy.wpost_dispute_window;

    // first, try to dispute right before the window end.
    // We expect this to fail "normally" (fail to disprove).
    rt.epoch = window_end - 1;
    h.dispute_window_post(&mut rt, &nextdl, 0, &infos, None);

    // Now set the epoch at the window end. We expect a different error.
    rt.epoch = window_end;

    // Now try to dispute.
    let params = miner::DisputeWindowedPoStParams { deadline: dlidx, post_index: 0 };
    rt.set_caller(*ACCOUNT_ACTOR_CODE_ID, h.worker);
    rt.expect_validate_caller_type((*CALLER_TYPES_SIGNABLE).to_vec());

    h.expect_query_network_info(&mut rt);

    let result = rt.call::<miner::Actor>(
        miner::Method::DisputeWindowedPoSt as u64,
        &RawBytes::serialize(params).unwrap(),
    );
    expect_abort_contains_message(
        ExitCode::USR_FORBIDDEN,
        "can only dispute window posts during the dispute window",
        result,
    );
    rt.verify();
}

#[test]
fn cant_dispute_up_with_an_invalid_deadline() {
    let period_offset = ChainEpoch::from(100);
    let precommit_epoch = ChainEpoch::from(1);

    let mut h = ActorHarness::new(period_offset);
    h.set_proof_type(RegisteredSealProof::StackedDRG2KiBV1P1);

    let mut rt = h.new_runtime();
    rt.epoch = precommit_epoch;
    rt.balance.replace(BIG_BALANCE.clone());

    h.construct_and_verify(&mut rt);

    let params = miner::DisputeWindowedPoStParams { deadline: 50, post_index: 0 };

    rt.set_caller(*ACCOUNT_ACTOR_CODE_ID, h.worker);
    rt.expect_validate_caller_type((*CALLER_TYPES_SIGNABLE).to_vec());

    let result = rt.call::<miner::Actor>(
        miner::Method::DisputeWindowedPoSt as u64,
        &RawBytes::serialize(params).unwrap(),
    );
    expect_abort_contains_message(ExitCode::USR_ILLEGAL_ARGUMENT, "invalid deadline", result);
    rt.verify();
}

#[test]
fn can_dispute_test_after_proving_period_changes() {
    let period_offset = ChainEpoch::from(100);
    let precommit_epoch = ChainEpoch::from(1);

    let mut h = ActorHarness::new(period_offset);
    h.set_proof_type(RegisteredSealProof::StackedDRG2KiBV1P1);

    let mut rt = h.new_runtime();
    rt.epoch = precommit_epoch;
    rt.balance.replace(BIG_BALANCE.clone());

    h.construct_and_verify(&mut rt);

    let period_start = h.deadline(&rt).next_period_start();

    // go to the next deadline 0
    rt.epoch = period_start;

    // fill one partition in each mutable deadline.
    let num_sectors = h.partition_size * (rt.policy.wpost_period_deadlines - 2);

    // creates a partition in every deadline except 0 and 47
    // TODO: when fixing last wpost test verify that this is true
    let sectors = h.commit_and_prove_sectors(
        &mut rt,
        num_sectors as usize,
        DEFAULT_SECTOR_EXPIRATION,
        vec![],
        true,
    );

    // prove every sector once to activate power. This
    // simplifies the test a bit.
    h.advance_and_submit_posts(&mut rt, &sectors);

    // Make sure we're in the correct deadline. We should
    // finish at deadline 2 because precommit takes some
    // time.
    let dlinfo = h.deadline(&rt);
    assert!(
        dlinfo.index < 46,
        "we need to be before the target deadline for this test to make sense"
    );

    // Now find the sectors in the last partition.
    let (_, partition) = h.get_deadline_and_partition(&rt, 46, 0);
    let mut target_sectors = Vec::new();
    for i in partition.sectors.iter() {
        for sector in sectors.iter() {
            if sector.sector_number == i {
                target_sectors.push(sector.clone());
            }
        }
    }
    assert!(!target_sectors.is_empty());

    let pwr = miner::power_for_sectors(h.sector_size, &target_sectors);

    // And challenge the last partition.
    let expected_fee = miner::pledge_penalty_for_invalid_windowpost(
        &h.epoch_reward_smooth,
        &h.epoch_qa_power_smooth,
        &pwr.qa,
    );
    let post_dispute_result = PoStDisputeResult {
        expected_power_delta: Some(-pwr),
        expected_penalty: Some(expected_fee),
        expected_reward: Some(miner::BASE_REWARD_FOR_DISPUTED_WINDOW_POST.clone()),
        expected_pledge_delta: None,
    };

    let target_dlinfo = miner::DeadlineInfo::new(
        period_start,
        46,
        rt.epoch,
        rt.policy.wpost_period_deadlines,
        rt.policy.wpost_proving_period,
        rt.policy.wpost_challenge_window,
        rt.policy.wpost_challenge_lookback,
        rt.policy.fault_declaration_cutoff,
    );

    h.dispute_window_post(&mut rt, &target_dlinfo, 0, &target_sectors, Some(post_dispute_result));
}

#[test]
fn bad_post_fails_when_verified() {
    let period_offset = ChainEpoch::from(100);
    let precommit_epoch = ChainEpoch::from(1);

    let mut h = ActorHarness::new(period_offset);
    h.set_proof_type(RegisteredSealProof::StackedDRG2KiBV1P1);

    let mut rt = h.new_runtime();
    rt.epoch = precommit_epoch;
    rt.balance.replace(BIG_BALANCE.clone());

    h.construct_and_verify(&mut rt);

    let infos = h.commit_and_prove_sectors(&mut rt, 2, DEFAULT_SECTOR_EXPIRATION, vec![], true);
    let power_for_sectors =
        &miner::power_for_sectors(h.sector_size, &vec![infos[0].clone(), infos[1].clone()]);

    h.apply_rewards(&mut rt, BIG_REWARDS.clone(), TokenAmount::zero());

    let state = h.get_state(&rt);
    let (dlidx, pidx) = state.find_sector(&rt.policy, &rt.store, infos[0].sector_number).unwrap();
    let (dlidx2, pidx2) = state.find_sector(&rt.policy, &rt.store, infos[1].sector_number).unwrap();
    assert_eq!(dlidx, dlidx2);
    assert_eq!(pidx, pidx2);

    // Become faulty

    h.advance_to_deadline(&mut rt, dlidx);
    h.advance_deadline(&mut rt, CronConfig::empty());
    h.advance_to_deadline(&mut rt, dlidx);

    let fault_fee = h.continued_fault_penalty(&vec![infos[0].clone(), infos[1].clone()]);
    h.advance_deadline(
        &mut rt,
        CronConfig::with_detected_faults_power_delta_and_continued_faults_penalty(
            &PowerPair::zero(),
            fault_fee,
        ),
    );

    // Promise to recover

    let mut bf = BitField::new();
    bf.set(infos[0].sector_number);
    bf.set(infos[1].sector_number);
    h.declare_recoveries(&mut rt, dlidx, pidx, bf, TokenAmount::zero()).unwrap();

    // Now submit a PoSt, but a BAD one
    let dlinfo = h.advance_to_deadline(&mut rt, dlidx);

    let partition = miner::PoStPartition { index: pidx, skipped: make_bitfield(&[]) };
    let mut post_config = PoStConfig::with_expected_power_delta(power_for_sectors);
    // this makes the PoSt BAD
    post_config.verification_exit = Some(ExitCode::USR_ILLEGAL_ARGUMENT);

    let params = miner::SubmitWindowedPoStParams {
        deadline: dlidx,
        partitions: vec![partition],
        proofs: make_post_proofs(h.window_post_proof_type),
        chain_commit_epoch: dlinfo.challenge,
        chain_commit_rand: Randomness(TEST_RANDOMNESS_ARRAY_FROM_ONE.into()),
    };
    let result = h.submit_window_post_raw(&mut rt, &dlinfo, infos, params, post_config);
    expect_abort_contains_message(
        ExitCode::USR_ILLEGAL_ARGUMENT,
        "invalid post was submitted",
        result,
    );

    h.check_state(&rt);
}
