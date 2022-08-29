use fil_actor_miner::{
    expected_reward_for_power, new_deadline_info, pledge_penalty_for_termination,
    qa_power_for_sector, State, INITIAL_PLEDGE_PROJECTION_PERIOD,
};
use fil_actors_runtime::{
    runtime::{Runtime, RuntimePolicy},
    test_utils::{expect_abort, expect_abort_contains_message, MockRuntime},
    EPOCHS_IN_DAY,
};
use fvm_ipld_bitfield::BitField;
use fvm_shared::{clock::ChainEpoch, econ::TokenAmount, error::ExitCode, sector::SectorNumber};

mod util;
use fvm_shared::bigint::{BigInt, Zero};
use itertools::Itertools;
use util::*;
const PERIOD_OFFSET: ChainEpoch = 100;

fn setup() -> (ActorHarness, MockRuntime) {
    let big_balance = 20u128.pow(23);

    let h = ActorHarness::new(PERIOD_OFFSET);
    let mut rt = h.new_runtime();
    h.construct_and_verify(&mut rt);
    rt.balance.replace(TokenAmount::from(big_balance));

    (h, rt)
}

fn assert_sectors_exists(
    rt: &MockRuntime,
    sector_number: SectorNumber,
    expected_partition: u64,
    expected_deadline: u64,
) {
    let state: State = rt.get_state();
    assert!(state.get_sector(rt.store(), sector_number).unwrap().is_some());

    let (deadline, pid) = state.find_sector(rt.policy(), rt.store(), sector_number).unwrap();
    assert_eq!(expected_partition, pid);
    assert_eq!(expected_deadline, deadline);
}

fn assert_sectors_not_found(rt: &MockRuntime, sector_number: SectorNumber) {
    let state: State = rt.get_state();
    assert!(state.get_sector(rt.store(), sector_number).unwrap().is_none());

    let err = state.find_sector(rt.policy(), rt.store(), sector_number).err().unwrap();
    assert!(err.to_string().contains("not due at any deadline"));
}

#[test]
fn compacting_a_partition_with_both_live_and_dead_sectors_removes_dead_sectors_retains_live_sectors(
) {
    let (mut h, mut rt) = setup();
    rt.set_epoch(200);

    // create 4 sectors in partition 0
    let sectors_info = h.commit_and_prove_sectors(
        &mut rt,
        4,
        DEFAULT_SECTOR_EXPIRATION,
        vec![vec![10], vec![20], vec![30], vec![40]],
        true,
    );

    h.advance_and_submit_posts(&mut rt, &sectors_info);

    assert_eq!(sectors_info.len(), 4);
    let sectors = sectors_info.iter().map(|info| info.sector_number).collect_vec();

    // terminate sector 1
    rt.set_epoch(rt.epoch + 100);
    h.apply_rewards(&mut rt, BIG_REWARDS.into(), BigInt::zero());

    let terminated_sector = &sectors_info[0];
    let sector_size = terminated_sector.seal_proof.sector_size().unwrap();
    let sector_power = qa_power_for_sector(sector_size, terminated_sector);
    let day_reward = expected_reward_for_power(
        &h.epoch_reward_smooth,
        &h.epoch_qa_power_smooth,
        &sector_power,
        EPOCHS_IN_DAY,
    );
    let twenty_day_reward = expected_reward_for_power(
        &h.epoch_reward_smooth,
        &h.epoch_qa_power_smooth,
        &sector_power,
        INITIAL_PLEDGE_PROJECTION_PERIOD,
    );
    let sector_age = rt.epoch - terminated_sector.activation;
    let expected_fee = pledge_penalty_for_termination(
        &day_reward,
        sector_age,
        &twenty_day_reward,
        &h.epoch_qa_power_smooth,
        &sector_power,
        &h.epoch_reward_smooth,
        &BigInt::zero(),
        0,
    );

    h.terminate_sectors(&mut rt, &bitfield_from_slice(&[sectors[0]]), expected_fee);

    // Wait WPoStProofChallengePeriod epochs so we can compact the sector.
    let target_epoch = rt.epoch + rt.policy().wpost_dispute_window;
    h.advance_to_epoch_with_cron(&mut rt, target_epoch);

    // compacting partition will remove sector 1 but retain sector 2,3 and 4
    let deadline_id = 0;
    let partition_id = 0;
    let partitions = bitfield_from_slice(&[partition_id]);
    h.compact_partitions(&mut rt, deadline_id, partitions).unwrap();

    assert_sectors_not_found(&rt, sectors[0]);
    assert_sectors_exists(&rt, sectors[1], partition_id, deadline_id);
    assert_sectors_exists(&rt, sectors[2], partition_id, deadline_id);
    assert_sectors_exists(&rt, sectors[3], partition_id, deadline_id);

    h.check_state(&rt);
}

#[test]
fn fail_to_compact_partitions_with_faults() {
    let (mut h, mut rt) = setup();
    rt.set_epoch(200);

    // create 2 sectors in partition 0
    let sectors_info = h.commit_and_prove_sectors(
        &mut rt,
        2,
        DEFAULT_SECTOR_EXPIRATION,
        vec![vec![10], vec![20]],
        true,
    );
    h.advance_and_submit_posts(&mut rt, &sectors_info);

    // fault sector 1
    h.declare_faults(&mut rt, &sectors_info[0..1]);

    // Wait WPoStProofChallengePeriod epochs so we can compact the sector.
    let target_epoch = rt.epoch + rt.policy().wpost_dispute_window;
    h.advance_to_epoch_with_cron(&mut rt, target_epoch);

    let partition_id = 0;
    let deadline_id = 0;

    let result = h.compact_partitions(&mut rt, deadline_id, bitfield_from_slice(&[partition_id]));
    expect_abort_contains_message(ExitCode::USR_ILLEGAL_ARGUMENT, "failed to remove partitions from deadline 0: while removing partitions: cannot remove partition 0: has faults", result);

    h.check_state(&rt);
}

#[test]
fn fails_to_compact_partitions_with_unproven_sectors() {
    let (mut h, mut rt) = setup();

    // Wait until deadline 0 (the one to which we'll assign the
    // sector) has elapsed. That'll let us commit, prove, then wait
    // finality epochs.
    let state: State = rt.get_state();
    let deadline_epoch = new_deadline_info(rt.policy(), state.proving_period_start, 0, rt.epoch)
        .next_not_elapsed()
        .next_open();
    rt.set_epoch(deadline_epoch);

    // create 2 sectors in partition 0
    h.commit_and_prove_sectors(
        &mut rt,
        2,
        DEFAULT_SECTOR_EXPIRATION,
        vec![vec![10], vec![20]],
        true,
    );

    // Wait WPoStProofChallengePeriod epochs so we can compact the sector.
    let target_epoch = rt.epoch + rt.policy().wpost_dispute_window;
    h.advance_to_epoch_with_cron(&mut rt, target_epoch);

    let partition_id = 0;
    let deadline_id = 0;

    let result = h.compact_partitions(&mut rt, deadline_id, bitfield_from_slice(&[partition_id]));
    expect_abort_contains_message(ExitCode::USR_ILLEGAL_ARGUMENT, "failed to remove partitions from deadline 0: while removing partitions: cannot remove partition 0: has unproven sectors", result);

    h.check_state(&rt);
}

#[test]
fn fails_if_deadline_out_of_range() {
    let (h, mut rt) = setup();
    let w_post_period_deadlines = rt.policy().wpost_period_deadlines;
    let result = h.compact_partitions(&mut rt, w_post_period_deadlines, BitField::default());
    expect_abort_contains_message(
        ExitCode::USR_ILLEGAL_ARGUMENT,
        &format!("invalid deadline {w_post_period_deadlines}"),
        result,
    );

    h.check_state(&rt);
}

#[test]
fn fails_if_deadline_is_open_for_challenging() {
    let (h, mut rt) = setup();
    rt.set_epoch(PERIOD_OFFSET);
    assert_eq!(h.current_deadline(&rt).index, 0);

    let result = h.compact_partitions(&mut rt, 0, BitField::default());
    expect_abort(ExitCode::USR_FORBIDDEN, result);

    h.check_state(&rt);
}

#[test]
fn fails_if_deadline_is_next_up_to_be_challenged() {
    let (h, mut rt) = setup();
    rt.set_epoch(PERIOD_OFFSET);
    let current_deadline = h.current_deadline(&rt).index;
    let result = h.compact_partitions(&mut rt, current_deadline + 1, BitField::default());
    expect_abort(ExitCode::USR_FORBIDDEN, result);

    h.check_state(&rt);
}

#[test]
fn deadline_after_next_deadline_should_still_be_open_for_compaction() {
    let (h, mut rt) = setup();
    rt.set_epoch(PERIOD_OFFSET);
    let current_deadline = h.current_deadline(&rt).index;
    h.compact_partitions(&mut rt, current_deadline + 2, BitField::default()).unwrap();
    h.check_state(&rt);
}

#[test]
fn deadlines_challenged_last_proving_period_should_still_be_in_dispute_window() {
    let (h, mut rt) = setup();
    rt.set_epoch(PERIOD_OFFSET);
    // (curr_deadline - 1) % wpost_period_deadlines
    let last_proving_period = rt.policy().wpost_period_deadlines - 1;
    let result = h.compact_partitions(&mut rt, last_proving_period, BitField::default());
    expect_abort(ExitCode::USR_FORBIDDEN, result);

    h.check_state(&rt);
}

#[test]
fn compaction_should_be_forbidden_during_the_dispute_window() {
    let (h, mut rt) = setup();

    let dispute_end =
        PERIOD_OFFSET + rt.policy().wpost_challenge_window + rt.policy().wpost_dispute_window - 1;
    rt.set_epoch(dispute_end);

    let result = h.compact_partitions(&mut rt, 0, BitField::default());
    expect_abort(ExitCode::USR_FORBIDDEN, result);

    h.check_state(&rt);
}

#[test]
fn compaction_should_be_allowed_following_the_dispute_window() {
    let (h, mut rt) = setup();

    let dispute_end =
        PERIOD_OFFSET + rt.policy().wpost_challenge_window + rt.policy().wpost_dispute_window - 1;
    rt.set_epoch(dispute_end + 1);

    h.compact_partitions(&mut rt, 0, BitField::default()).unwrap();

    h.check_state(&rt);
}

#[test]
fn fails_if_partition_count_is_above_limit() {
    let (h, mut rt) = setup();

    // partition limit is 4 for the default construction
    let partitions = bitfield_from_slice(&[1, 2, 3, 4, 5]);

    let result = h.compact_partitions(&mut rt, 1, partitions);
    expect_abort(ExitCode::USR_ILLEGAL_ARGUMENT, result);

    h.check_state(&rt);
}
