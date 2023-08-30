use fil_actor_miner::{
    deadline_available_for_compaction, deadline_available_for_optimistic_post_dispute,
    deadline_is_mutable, new_deadline_info, DeadlineInfo,
};
use fil_actors_runtime::{
    runtime::{DomainSeparationTag, RuntimePolicy},
    test_utils::{expect_abort_contains_message, MockRuntime},
};
use fvm_ipld_bitfield::BitField;
use fvm_ipld_encoding::RawBytes;
use fvm_shared::randomness::Randomness;
use fvm_shared::{clock::ChainEpoch, error::ExitCode};

mod util;
use util::*;
const PERIOD_OFFSET: ChainEpoch = 100;

fn setup() -> (ActorHarness, MockRuntime) {
    let h = ActorHarness::new(PERIOD_OFFSET);
    let rt = h.new_runtime();
    h.construct_and_verify(&rt);
    rt.balance.replace(BIG_BALANCE.clone());

    (h, rt)
}

// returns the nearest epoch such that synchronous post verification is required
fn nearest_unsafe_epoch(rt: &MockRuntime, h: &ActorHarness, from_deadline_id: u64) -> i64 {
    let current_ddl = h.current_deadline(rt);

    for i in *rt.epoch.borrow().. {
        if !deadline_available_for_compaction(
            &rt.policy,
            current_ddl.period_start,
            from_deadline_id,
            i,
        ) && deadline_available_for_optimistic_post_dispute(
            &rt.policy,
            current_ddl.period_start,
            from_deadline_id,
            i,
        ) {
            return i;
        }
    }

    panic!("impossible path");
}

// returns the nearest epoch such that no synchronous post verification is necessary
fn nearest_safe_epoch(rt: &MockRuntime, h: &ActorHarness, from_deadline_id: u64) -> i64 {
    let current_ddl = h.current_deadline(rt);

    for i in *rt.epoch.borrow().. {
        if deadline_available_for_compaction(
            &rt.policy,
            current_ddl.period_start,
            from_deadline_id,
            i,
        ) {
            return i;
        }
    }

    panic!("impossible path");
}

// returns the farthest deadline from current that satisfies deadline_available_for_move
fn farthest_possible_to_deadline(
    rt: &MockRuntime,
    from_deadline_id: u64,
    current_deadline: DeadlineInfo,
) -> u64 {
    assert_ne!(
        from_deadline_id, current_deadline.index,
        "can't move nearer when the deadline_distance is 0"
    );

    if current_deadline.index < from_deadline_id {
        // the deadline distance can only be nearer
        for i in (current_deadline.index..(from_deadline_id)).rev() {
            if deadline_is_mutable(&rt.policy, current_deadline.period_start, i, *rt.epoch.borrow())
            {
                return i;
            }
        }
    } else {
        for i in (0..(from_deadline_id)).rev() {
            if deadline_is_mutable(&rt.policy, current_deadline.period_start, i, *rt.epoch.borrow())
            {
                return i;
            }
        }

        for i in (current_deadline.index..rt.policy.wpost_period_deadlines).rev() {
            if deadline_is_mutable(&rt.policy, current_deadline.period_start, i, *rt.epoch.borrow())
            {
                return i;
            }
        }
    }

    panic!("no candidate to_deadline");
}

#[test]
fn fail_to_move_partitions_with_faults_from_safe_epoch() {
    let (mut h, rt) = setup();
    rt.set_epoch(200);

    // create 2 sectors in partition 0
    let sectors_info = h.commit_and_prove_sectors(
        &rt,
        2,
        DEFAULT_SECTOR_EXPIRATION,
        vec![vec![10], vec![20]],
        true,
    );
    h.advance_and_submit_posts(&rt, &sectors_info);

    // fault sector 1
    h.declare_faults(&rt, &sectors_info[0..1]);

    let partition_id = 0;
    let from_deadline_id = 0;

    h.advance_to_epoch_with_cron(&rt, nearest_safe_epoch(&rt, &h, from_deadline_id));

    let to_deadline_id =
        farthest_possible_to_deadline(&rt, from_deadline_id, h.current_deadline(&rt));

    let result = h.move_partitions(
        &rt,
        from_deadline_id,
        to_deadline_id,
        bitfield_from_slice(&[partition_id]),
        || {},
    );
    expect_abort_contains_message(
        ExitCode::USR_ILLEGAL_STATE,
        "partition with faults or unproven sectors are not allowed to move",
        result,
    );

    h.check_state(&rt);
}

#[test]
fn fail_to_move_partitions_with_faults_from_unsafe_epoch() {
    let (mut h, rt) = setup();
    rt.set_epoch(200);

    // create 2 sectors in partition 0
    let sectors_info = h.commit_and_prove_sectors(
        &rt,
        2,
        DEFAULT_SECTOR_EXPIRATION,
        vec![vec![10], vec![20]],
        true,
    );
    h.advance_and_submit_posts(&rt, &sectors_info);

    // fault sector 1
    h.declare_faults(&rt, &sectors_info[0..1]);

    let partition_id = 0;
    let from_deadline_id = 0;

    h.advance_to_epoch_with_cron(&rt, nearest_unsafe_epoch(&rt, &h, from_deadline_id));

    let to_deadline_id =
        farthest_possible_to_deadline(&rt, from_deadline_id, h.current_deadline(&rt));

    let result = h.move_partitions(
        &rt,
        from_deadline_id,
        to_deadline_id,
        bitfield_from_slice(&[partition_id]),
        || {
            let current_deadline = h.current_deadline(&rt);

            let from_deadline = new_deadline_info(
                rt.policy(),
                if current_deadline.index < from_deadline_id {
                    current_deadline.period_start - rt.policy().wpost_proving_period
                } else {
                    current_deadline.period_start
                },
                from_deadline_id,
                *rt.epoch.borrow(),
            );

            let from_ddl = h.get_deadline(&rt, from_deadline_id);

            let entropy = RawBytes::serialize(h.receiver).unwrap();
            rt.expect_get_randomness_from_beacon(
                DomainSeparationTag::WindowedPoStChallengeSeed,
                from_deadline.challenge,
                entropy.to_vec(),
                TEST_RANDOMNESS_ARRAY_FROM_ONE,
            );

            let post = h.get_submitted_proof(&rt, &from_ddl, 0);

            let all_ignored = BitField::new();
            let vi = h.make_window_post_verify_info(
                &sectors_info,
                &all_ignored,
                sectors_info[1].clone(),
                Randomness(TEST_RANDOMNESS_ARRAY_FROM_ONE.into()),
                post.proofs,
            );
            rt.expect_verify_post(vi, ExitCode::OK);
        },
    );
    expect_abort_contains_message(
        ExitCode::USR_ILLEGAL_STATE,
        "partition with faults or unproven sectors are not allowed to move",
        result,
    );

    h.check_state(&rt);
}

#[test]
fn ok_to_move_partitions_from_safe_epoch() {
    let (mut h, rt) = setup();
    rt.set_epoch(200);

    // create 2 sectors in partition 0
    let sectors_info = h.commit_and_prove_sectors(
        &rt,
        2,
        DEFAULT_SECTOR_EXPIRATION,
        vec![vec![10], vec![20]],
        true,
    );
    h.advance_and_submit_posts(&rt, &sectors_info);

    let from_deadline_id = 0;

    h.advance_to_epoch_with_cron(&rt, nearest_safe_epoch(&rt, &h, from_deadline_id));

    let partition_id = 0;
    let to_deadline_id =
        farthest_possible_to_deadline(&rt, from_deadline_id, h.current_deadline(&rt));

    let result = h.move_partitions(
        &rt,
        from_deadline_id,
        to_deadline_id,
        bitfield_from_slice(&[partition_id]),
        || {},
    );
    assert!(result.is_ok());

    h.check_state(&rt);
}

#[test]
fn ok_to_move_partitions_from_unsafe_epoch() {
    let (mut h, rt) = setup();
    rt.set_epoch(200);

    // create 2 sectors in partition 0
    let sectors_info = h.commit_and_prove_sectors(
        &rt,
        2,
        DEFAULT_SECTOR_EXPIRATION,
        vec![vec![10], vec![20]],
        true,
    );
    h.advance_and_submit_posts(&rt, &sectors_info);

    let from_deadline_id = 0;

    h.advance_to_epoch_with_cron(&rt, nearest_unsafe_epoch(&rt, &h, from_deadline_id));

    let partition_id = 0;
    let to_deadline_id =
        farthest_possible_to_deadline(&rt, from_deadline_id, h.current_deadline(&rt));

    let result = h.move_partitions(
        &rt,
        from_deadline_id,
        to_deadline_id,
        bitfield_from_slice(&[partition_id]),
        || {
            let current_deadline = h.current_deadline(&rt);

            let from_deadline = new_deadline_info(
                rt.policy(),
                if current_deadline.index < from_deadline_id {
                    current_deadline.period_start - rt.policy().wpost_proving_period
                } else {
                    current_deadline.period_start
                },
                from_deadline_id,
                *rt.epoch.borrow(),
            );

            let from_ddl = h.get_deadline(&rt, from_deadline_id);

            let entropy = RawBytes::serialize(h.receiver).unwrap();
            rt.expect_get_randomness_from_beacon(
                DomainSeparationTag::WindowedPoStChallengeSeed,
                from_deadline.challenge,
                entropy.to_vec(),
                TEST_RANDOMNESS_ARRAY_FROM_ONE,
            );

            let post = h.get_submitted_proof(&rt, &from_ddl, 0);

            let all_ignored = BitField::new();
            let vi = h.make_window_post_verify_info(
                &sectors_info,
                &all_ignored,
                sectors_info[1].clone(),
                Randomness(TEST_RANDOMNESS_ARRAY_FROM_ONE.into()),
                post.proofs,
            );
            rt.expect_verify_post(vi, ExitCode::OK);
        },
    );
    assert!(result.is_ok());

    h.check_state(&rt);
}
