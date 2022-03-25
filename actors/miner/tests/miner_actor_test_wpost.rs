use fil_actor_miner as miner;
use fil_actors_runtime::test_utils::*;

use fvm_shared::clock::ChainEpoch;
use fvm_shared::crypto::randomness::DomainSeparationTag;
use fvm_shared::econ::TokenAmount;
use fvm_shared::encoding::RawBytes;
use fvm_shared::error::ExitCode;
use fvm_shared::randomness::Randomness;
use fvm_shared::sector::RegisteredPoStProof;
use fvm_shared::sector::RegisteredSealProof;

mod util;
use util::*;

// an expriration ~10 days greater than effective min expiration taking into account 30 days max
// between pre and prove commit
const DEFAULT_SECTOR_EXPIRATION: u64 = 220;

const BIG_BALANCE: u128 = 1_000_000_000_000_000_000_000_000u128;

#[test]
fn basic_post_and_dispute() {
    let period_offset = ChainEpoch::from(100);
    let precommit_epoch = ChainEpoch::from(1);

    let mut h = ActorHarness::new(period_offset);
    h.set_proof_type(RegisteredSealProof::StackedDRG2KiBV1P1);

    let mut rt = h.new_runtime();
    rt.epoch = precommit_epoch;
    rt.balance.replace(TokenAmount::from(BIG_BALANCE));

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
    check_state_invariants(&rt);

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
    rt.balance.replace(TokenAmount::from(BIG_BALANCE));

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
            chain_commit_rand: Randomness(b"chaincommitment".to_vec()),
        };
        let result = h.submit_window_post_raw(
            &mut rt,
            &dlinfo,
            vec![sector.clone()],
            params,
            PoStConfig::empty(),
        );
        expect_abort_contains_message(ExitCode::ErrIllegalArgument, "invalid deadline", result);
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
            chain_commit_rand: Randomness(b"chaincommitment".to_vec()),
        };
        let result = h.submit_window_post_raw(
            &mut rt,
            &dlinfo,
            vec![sector.clone()],
            params,
            PoStConfig::empty(),
        );
        expect_abort_contains_message(
            ExitCode::ErrIllegalArgument,
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
            chain_commit_rand: Randomness(b"chaincommitment".to_vec()),
        };
        let result = h.submit_window_post_raw(
            &mut rt,
            &dlinfo,
            vec![sector.clone()],
            params,
            PoStConfig::empty(),
        );
        expect_abort_contains_message(ExitCode::ErrIllegalArgument, "too many partitions", result);
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
            chain_commit_rand: Randomness(b"chaincommitment".to_vec()),
        };
        let result = h.submit_window_post_raw(
            &mut rt,
            &dlinfo,
            vec![sector.clone()],
            params,
            PoStConfig::empty(),
        );
        expect_abort_contains_message(ExitCode::ErrNotFound, "no such partition", result);
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
            chain_commit_rand: Randomness(b"chaincommitment".to_vec()),
        };
        let result = h.submit_window_post_raw(
            &mut rt,
            &dlinfo,
            vec![sector.clone()],
            params,
            PoStConfig::empty(),
        );
        expect_abort_contains_message(
            ExitCode::ErrIllegalArgument,
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
            chain_commit_rand: Randomness(b"chaincommitment".to_vec()),
        };
        let result = h.submit_window_post_raw(
            &mut rt,
            &dlinfo,
            vec![sector.clone()],
            params,
            PoStConfig::empty(),
        );
        expect_abort_contains_message(
            ExitCode::ErrIllegalArgument,
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
            chain_commit_rand: Randomness(b"chaincommitment".to_vec()),
        };
        let result = h.submit_window_post_raw(
            &mut rt,
            &dlinfo,
            vec![sector.clone()],
            params,
            PoStConfig::empty(),
        );
        expect_abort_contains_message(
            ExitCode::ErrIllegalArgument,
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
            chain_commit_rand: Randomness(b"chaincommitment".to_vec()),
        };
        let result = h.submit_window_post_raw(
            &mut rt,
            &dlinfo,
            vec![sector.clone()],
            params,
            PoStConfig::empty(),
        );
        expect_abort_contains_message(
            ExitCode::ErrIllegalArgument,
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
            chain_commit_rand: Randomness(b"chaincommitment".to_vec()),
        };
        let result = h.submit_window_post_raw(
            &mut rt,
            &dlinfo,
            vec![sector.clone()],
            params,
            PoStConfig::empty(),
        );
        expect_abort_contains_message(
            ExitCode::ErrIllegalArgument,
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
        expect_abort_contains_message(ExitCode::ErrIllegalArgument, "bytes of randomness", result);
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
            chain_commit_rand: Randomness(b"chaincommitment".to_vec()),
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
        expect_abort_contains_message(ExitCode::ErrIllegalArgument, "invalid deadline", result);
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
            chain_commit_rand: Randomness(b"chaincommitment".to_vec()),
        };
        let result = h.submit_window_post_raw(
            &mut rt,
            &dlinfo,
            vec![sector.clone()],
            params,
            PoStConfig::empty(),
        );
        expect_abort_contains_message(
            ExitCode::ErrIllegalArgument,
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
            chain_commit_rand: Randomness(b"chaincommitment".to_vec()),
        };
        let result = h.submit_window_post_raw(
            &mut rt,
            &dlinfo,
            vec![sector.clone()],
            params,
            PoStConfig::empty(),
        );
        expect_abort_contains_message(
            ExitCode::ErrIllegalArgument,
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
            chain_commit_rand: Randomness(b"boo".to_vec()),
        };
        let result = h.submit_window_post_raw(
            &mut rt,
            &dlinfo,
            vec![sector.clone()],
            params,
            PoStConfig::with_randomness(Randomness(b"far".to_vec())),
        );
        expect_abort_contains_message(
            ExitCode::ErrIllegalArgument,
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
            chain_commit_rand: Randomness(b"chaincommitment".to_vec()),
        };
        let result = h.submit_window_post_raw(
            &mut rt,
            &dlinfo,
            vec![sector],
            params,
            PoStConfig::with_expected_power_delta(&pwr),
        );
        expect_ok(result);
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
    rt.balance.replace(TokenAmount::from(BIG_BALANCE));

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
    let commit_rand = Randomness(b"chaincommitment".to_vec());
    let partition =
        miner::PoStPartition { index: pidx, skipped: make_bitfield(&[sector.sector_number]) };
    let params = miner::SubmitWindowedPoStParams {
        deadline: dlidx,
        partitions: vec![partition],
        proofs: make_post_proofs(h.window_post_proof_type),
        chain_commit_epoch: dlinfo.challenge,
        chain_commit_rand: commit_rand.clone(),
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
    expect_abort_contains_message(ExitCode::ErrIllegalArgument, "partition already proven", result);
    rt.reset();

    // Advance to end-of-deadline cron to verify no penalties.
    h.advance_deadline(&mut rt, CronConfig::empty());
    check_state_invariants(&rt);
}

#[test]
fn duplicate_proof_rejected_with_many_partitions() {
    let period_offset = ChainEpoch::from(100);
    let precommit_epoch = ChainEpoch::from(1);

    let mut h = ActorHarness::new(period_offset);
    h.set_proof_type(RegisteredSealProof::StackedDRG2KiBV1P1);

    let mut rt = h.new_runtime();
    rt.epoch = precommit_epoch;
    rt.balance.replace(TokenAmount::from(BIG_BALANCE));

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
            chain_commit_rand: Randomness(b"chaincommitment".to_vec()),
        };
        let result = h.submit_window_post_raw(
            &mut rt,
            &dlinfo,
            sectors_to_prove,
            params,
            PoStConfig::with_expected_power_delta(&pwr),
        );
        expect_abort_contains_message(
            ExitCode::ErrIllegalArgument,
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
    check_state_invariants(&rt);
}

#[test]
fn successful_recoveries_recover_power() {}

#[test]
fn skipped_faults_adjust_power() {}

#[test]
fn skipping_all_sectors_in_a_partition_rejected() {}

#[test]
fn skipped_recoveries_are_penalized_and_do_not_recover_power() {}

#[test]
fn skipping_a_fault_from_the_wrong_partition_is_an_error() {}

#[test]
fn cannot_dispute_posts_when_the_challenge_window_is_open() {}

#[test]
fn can_dispute_up_till_window_end_but_not_after() {}

#[test]
fn cant_dispute_up_with_an_invalid_deadline() {}

#[test]
fn can_dispute_test_after_proving_period_changes() {}
