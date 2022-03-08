use fil_actor_miner as miner;

use fvm_shared::clock::ChainEpoch;
use fvm_shared::econ::TokenAmount;
use fvm_shared::sector::RegisteredSealProof;

use fvm_ipld_bitfield::{BitField, UnvalidatedBitField};

mod util;
use util::*;

// an expriration ~10 days greater than effective min expiration taking into account 30 days max
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
    rt.balance.replace(TokenAmount::from(1_000_000_000_000_000_000_000_000i128));

    h.construct_and_verify(&mut rt);

    let sectors = h.commit_and_prove_sectors(&mut rt, 1, DEFAULT_SECTOR_EXPIRATION, vec![], true);
    let sector = sectors[0].clone();
    let pwr = miner::power_for_sector(h.sector_size, &sector);

    // Skip to the right deadline
    let state = h.get_state(&rt);
    let (dlidx, pidx) = state.find_sector(&rt.policy, &rt.store, sector.sector_number).unwrap();
    let dlinfo = h.advance_to_deadline(&mut rt, dlidx);

    // Submit PoSt
    let post_partitions = vec![miner::PoStPartition {
        index: pidx,
        skipped: UnvalidatedBitField::Validated(BitField::new()),
    }];
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
    let dispute_sectors = vec![sector.clone()];
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
fn invalid_submissions() {}

#[test]
fn duplicate_proof_rejected() {}

#[test]
fn duplicate_proof_rejected_with_many_partitions() {}

#[test]
fn successful_recoveries_recover_power() {}

#[test]
fn skippled_faults_adjust_power() {}

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
