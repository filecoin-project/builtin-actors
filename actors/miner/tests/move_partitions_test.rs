use fil_actor_miner::{
    deadline_available_for_compaction, deadline_available_for_optimistic_post_dispute,
    deadline_is_mutable, expected_reward_for_power, new_deadline_info,
    pledge_penalty_for_termination, qa_power_for_sector, DeadlineInfo, SectorOnChainInfo, State,
    INITIAL_PLEDGE_PROJECTION_PERIOD,
};

use fil_actors_runtime::network::EPOCHS_IN_DAY;
use fil_actors_runtime::{
    runtime::Runtime,
    runtime::{DomainSeparationTag, RuntimePolicy},
    test_utils::{expect_abort_contains_message, MockRuntime},
};
use fvm_ipld_bitfield::BitField;
use fvm_ipld_encoding::RawBytes;
use fvm_shared::econ::TokenAmount;
use fvm_shared::randomness::Randomness;
use fvm_shared::{clock::ChainEpoch, error::ExitCode};
use num_traits::Zero;

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
fn nearest_unsafe_epoch(rt: &MockRuntime, h: &ActorHarness, orig_deadline_id: u64) -> i64 {
    let current_ddl = h.current_deadline(rt);

    for i in *rt.epoch.borrow().. {
        if !deadline_available_for_compaction(
            &rt.policy,
            current_ddl.period_start,
            orig_deadline_id,
            i,
        ) && deadline_available_for_optimistic_post_dispute(
            &rt.policy,
            current_ddl.period_start,
            orig_deadline_id,
            i,
        ) {
            return i;
        }
    }

    panic!("impossible path");
}

// returns the nearest epoch such that no synchronous post verification is necessary
fn nearest_safe_epoch(rt: &MockRuntime, h: &ActorHarness, orig_deadline_id: u64) -> i64 {
    let current_ddl = h.current_deadline(rt);

    for i in *rt.epoch.borrow().. {
        if deadline_available_for_compaction(
            &rt.policy,
            current_ddl.period_start,
            orig_deadline_id,
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
    orig_deadline_id: u64,
    current_deadline: DeadlineInfo,
) -> u64 {
    assert_ne!(
        orig_deadline_id, current_deadline.index,
        "can't move nearer when the deadline_distance is 0"
    );

    if current_deadline.index < orig_deadline_id {
        // the deadline distance can only be nearer
        for i in (current_deadline.index..(orig_deadline_id)).rev() {
            if deadline_is_mutable(&rt.policy, current_deadline.period_start, i, *rt.epoch.borrow())
            {
                return i;
            }
        }
    } else {
        for i in (0..(orig_deadline_id)).rev() {
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

    // create 2 sectors in partition 0
    let sectors_info = h.commit_and_prove_sectors(
        &rt,
        2,
        DEFAULT_SECTOR_EXPIRATION,
        vec![vec![10], vec![20]],
        true,
    );
    h.advance_and_submit_posts(&rt, &sectors_info);

    let st = h.get_state(&rt);
    let (orig_deadline_id, partition_id) =
        st.find_sector(&rt.store, sectors_info[0].sector_number).unwrap();

    // fault sector 1
    h.declare_faults(&rt, &sectors_info[0..1]);

    h.advance_to_epoch_with_cron(&rt, nearest_safe_epoch(&rt, &h, orig_deadline_id));

    let to_deadline_id =
        farthest_possible_to_deadline(&rt, orig_deadline_id, h.current_deadline(&rt));

    let result = h.move_partitions(
        &rt,
        orig_deadline_id,
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

    // create 2 sectors in partition 0
    let sectors_info = h.commit_and_prove_sectors(
        &rt,
        2,
        DEFAULT_SECTOR_EXPIRATION,
        vec![vec![10], vec![20]],
        true,
    );
    h.advance_and_submit_posts(&rt, &sectors_info);

    let st = h.get_state(&rt);
    let (orig_deadline_id, partition_id) =
        st.find_sector(&rt.store, sectors_info[0].sector_number).unwrap();

    // fault sector 1
    h.declare_faults(&rt, &sectors_info[0..1]);

    h.advance_to_epoch_with_cron(&rt, nearest_unsafe_epoch(&rt, &h, orig_deadline_id));

    let dest_deadline_id =
        farthest_possible_to_deadline(&rt, orig_deadline_id, h.current_deadline(&rt));

    let result = h.move_partitions(
        &rt,
        orig_deadline_id,
        dest_deadline_id,
        bitfield_from_slice(&[partition_id]),
        || {
            let current_deadline = h.current_deadline(&rt);

            let from_deadline = new_deadline_info(
                rt.policy(),
                if current_deadline.index < orig_deadline_id {
                    current_deadline.period_start - rt.policy().wpost_proving_period
                } else {
                    current_deadline.period_start
                },
                orig_deadline_id,
                *rt.epoch.borrow(),
            );

            let from_ddl = h.get_deadline(&rt, orig_deadline_id);

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

    // create 2 sectors in partition 0
    let sectors_info = h.commit_and_prove_sectors(
        &rt,
        2,
        DEFAULT_SECTOR_EXPIRATION,
        vec![vec![10], vec![20]],
        true,
    );
    h.advance_and_submit_posts(&rt, &sectors_info);

    let st = h.get_state(&rt);
    let (orig_deadline_id, partition_id) =
        st.find_sector(&rt.store, sectors_info[0].sector_number).unwrap();

    h.advance_to_epoch_with_cron(&rt, nearest_safe_epoch(&rt, &h, orig_deadline_id));

    let dest_deadline_id =
        farthest_possible_to_deadline(&rt, orig_deadline_id, h.current_deadline(&rt));

    let result = h.move_partitions(
        &rt,
        orig_deadline_id,
        dest_deadline_id,
        bitfield_from_slice(&[partition_id]),
        || {},
    );
    assert!(result.is_ok());

    h.check_state(&rt);
}

#[test]
fn ok_to_move_partitions_from_unsafe_epoch() {
    let (mut h, rt) = setup();

    // create 2 sectors in partition 0
    let sectors_info = h.commit_and_prove_sectors(
        &rt,
        2,
        DEFAULT_SECTOR_EXPIRATION,
        vec![vec![10], vec![20]],
        true,
    );
    h.advance_and_submit_posts(&rt, &sectors_info);

    let st = h.get_state(&rt);
    let (orig_deadline_id, partition_id) =
        st.find_sector(&rt.store, sectors_info[0].sector_number).unwrap();

    h.advance_to_epoch_with_cron(&rt, nearest_unsafe_epoch(&rt, &h, orig_deadline_id));

    let dest_deadline_id =
        farthest_possible_to_deadline(&rt, orig_deadline_id, h.current_deadline(&rt));

    let result = h.move_partitions(
        &rt,
        orig_deadline_id,
        dest_deadline_id,
        bitfield_from_slice(&[partition_id]),
        || {
            let current_deadline = h.current_deadline(&rt);

            let from_deadline = new_deadline_info(
                rt.policy(),
                if current_deadline.index < orig_deadline_id {
                    current_deadline.period_start - rt.policy().wpost_proving_period
                } else {
                    current_deadline.period_start
                },
                orig_deadline_id,
                *rt.epoch.borrow(),
            );

            let from_ddl = h.get_deadline(&rt, orig_deadline_id);

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

#[test]
fn fault_and_recover_after_move() {
    let (mut h, rt) = setup();

    let sectors_info = h.commit_and_prove_sectors(
        &rt,
        2,
        DEFAULT_SECTOR_EXPIRATION,
        vec![vec![10], vec![20]],
        true,
    );
    h.advance_and_submit_posts(&rt, &sectors_info);

    let st = h.get_state(&rt);
    let (orig_deadline_id, partition_id) =
        st.find_sector(&rt.store, sectors_info[0].sector_number).unwrap();

    h.advance_to_epoch_with_cron(&rt, nearest_safe_epoch(&rt, &h, orig_deadline_id));
    let dest_deadline_id =
        farthest_possible_to_deadline(&rt, orig_deadline_id, h.current_deadline(&rt));

    let result = h.move_partitions(
        &rt,
        orig_deadline_id,
        dest_deadline_id,
        bitfield_from_slice(&[partition_id]),
        || {},
    );
    assert!(result.is_ok());

    let st = h.get_state(&rt);
    let (dl_idx, p_idx) = st.find_sector(&rt.store, sectors_info[0].sector_number).unwrap();
    assert!(dl_idx == dest_deadline_id);

    h.check_state(&rt);

    // fault and recover

    h.declare_faults(&rt, &sectors_info);

    h.declare_recoveries(
        &rt,
        dl_idx,
        p_idx,
        BitField::try_from_bits(sectors_info.iter().map(|s| s.sector_number)).unwrap(),
        TokenAmount::zero(),
    )
    .unwrap();

    let dl = h.get_deadline(&rt, dl_idx);
    let p = dl.load_partition(&rt.store, p_idx).unwrap();
    assert_eq!(p.faults, p.recoveries);
    h.check_state(&rt);
}

#[test]
fn fault_and_terminate_after_move() {
    let (mut h, rt) = setup();

    let sectors_info = h.commit_and_prove_sectors(
        &rt,
        1,
        DEFAULT_SECTOR_EXPIRATION,
        vec![vec![10], vec![20]],
        true,
    );
    h.advance_and_submit_posts(&rt, &sectors_info);

    let st = h.get_state(&rt);
    let (orig_deadline_id, partition_id) =
        st.find_sector(&rt.store, sectors_info[0].sector_number).unwrap();

    h.advance_to_epoch_with_cron(&rt, nearest_safe_epoch(&rt, &h, orig_deadline_id));
    let dest_deadline_id =
        farthest_possible_to_deadline(&rt, orig_deadline_id, h.current_deadline(&rt));

    let result = h.move_partitions(
        &rt,
        orig_deadline_id,
        dest_deadline_id,
        bitfield_from_slice(&[partition_id]),
        || {},
    );
    assert!(result.is_ok());

    let st = h.get_state(&rt);
    let (dl_idx, _) = st.find_sector(&rt.store, sectors_info[0].sector_number).unwrap();
    assert!(dl_idx == dest_deadline_id);

    h.check_state(&rt);

    // fault and terminate

    h.declare_faults(&rt, &sectors_info);

    // A miner will pay the minimum of termination fee and locked funds. Add some locked funds to ensure
    // correct fee calculation is used.
    h.apply_rewards(&rt, BIG_REWARDS.clone(), TokenAmount::zero());
    let state: State = rt.get_state();
    let initial_locked_funds = state.locked_funds;

    let expected_fee = calc_expected_fee_for_termination(&h, &rt, sectors_info[0].clone());
    let sectors = bitfield_from_slice(&[sectors_info[0].sector_number]);
    h.terminate_sectors(&rt, &sectors, expected_fee.clone());

    // expect sector to be marked as terminated and the early termination queue to be empty (having been fully processed)
    let state: State = rt.get_state();
    let (_, mut partition) = h.find_sector(&rt, sectors_info[0].sector_number);
    let terminated = partition.terminated.get(sectors_info[0].sector_number);
    assert!(terminated);

    let (result, _) = partition.pop_early_terminations(rt.store(), 1000).unwrap();
    assert!(result.is_empty());

    // expect fee to have been unlocked and burnt
    assert_eq!(initial_locked_funds - expected_fee, state.locked_funds);

    //expect pledge requirement to have been decremented
    assert!(state.initial_pledge.is_zero());

    h.check_state(&rt);
}

fn calc_expected_fee_for_termination(
    h: &ActorHarness,
    rt: &MockRuntime,
    sector: SectorOnChainInfo,
) -> TokenAmount {
    let sector_power = qa_power_for_sector(sector.seal_proof.sector_size().unwrap(), &sector);
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
    let sector_age = *rt.epoch.borrow() - sector.activation;
    pledge_penalty_for_termination(
        &day_reward,
        sector_age,
        &twenty_day_reward,
        &h.epoch_qa_power_smooth,
        &sector_power,
        &h.epoch_reward_smooth,
        &TokenAmount::zero(),
        0,
    )
}

#[test]
fn directly_terminate_after_move() {
    let (mut h, rt) = setup();

    let sectors_info = h.commit_and_prove_sectors(
        &rt,
        1,
        DEFAULT_SECTOR_EXPIRATION,
        vec![vec![10], vec![20]],
        true,
    );
    h.advance_and_submit_posts(&rt, &sectors_info);

    let st = h.get_state(&rt);
    let (orig_deadline_id, partition_id) =
        st.find_sector(&rt.store, sectors_info[0].sector_number).unwrap();

    h.advance_to_epoch_with_cron(&rt, nearest_safe_epoch(&rt, &h, orig_deadline_id));
    let dest_deadline_id =
        farthest_possible_to_deadline(&rt, orig_deadline_id, h.current_deadline(&rt));

    let result = h.move_partitions(
        &rt,
        orig_deadline_id,
        dest_deadline_id,
        bitfield_from_slice(&[partition_id]),
        || {},
    );
    assert!(result.is_ok());

    let st = h.get_state(&rt);
    let (dl_idx, _) = st.find_sector(&rt.store, sectors_info[0].sector_number).unwrap();
    assert!(dl_idx == dest_deadline_id);

    h.check_state(&rt);

    // directly terminate

    // A miner will pay the minimum of termination fee and locked funds. Add some locked funds to ensure
    // correct fee calculation is used.
    h.apply_rewards(&rt, BIG_REWARDS.clone(), TokenAmount::zero());
    let state: State = rt.get_state();
    let initial_locked_funds = state.locked_funds;

    let expected_fee = calc_expected_fee_for_termination(&h, &rt, sectors_info[0].clone());
    let sectors = bitfield_from_slice(&[sectors_info[0].sector_number]);
    h.terminate_sectors(&rt, &sectors, expected_fee.clone());

    // expect sector to be marked as terminated and the early termination queue to be empty (having been fully processed)
    let state: State = rt.get_state();
    let (_, mut partition) = h.find_sector(&rt, sectors_info[0].sector_number);
    let terminated = partition.terminated.get(sectors_info[0].sector_number);
    assert!(terminated);

    let (result, _) = partition.pop_early_terminations(rt.store(), 1000).unwrap();
    assert!(result.is_empty());

    // expect fee to have been unlocked and burnt
    assert_eq!(initial_locked_funds - expected_fee, state.locked_funds);

    //expect pledge requirement to have been decremented
    assert!(state.initial_pledge.is_zero());

    h.check_state(&rt);
}

#[test]
fn fault_and_expire_after_move() {
    let (mut h, rt) = setup();

    let sectors_info = h.commit_and_prove_sectors(
        &rt,
        1,
        DEFAULT_SECTOR_EXPIRATION,
        vec![vec![10], vec![20]],
        true,
    );
    h.advance_and_submit_posts(&rt, &sectors_info);

    let st = h.get_state(&rt);
    let (orig_deadline_id, partition_id) =
        st.find_sector(&rt.store, sectors_info[0].sector_number).unwrap();

    h.advance_to_epoch_with_cron(&rt, nearest_safe_epoch(&rt, &h, orig_deadline_id));
    let dest_deadline_id =
        farthest_possible_to_deadline(&rt, orig_deadline_id, h.current_deadline(&rt));

    let result = h.move_partitions(
        &rt,
        orig_deadline_id,
        dest_deadline_id,
        bitfield_from_slice(&[partition_id]),
        || {},
    );
    assert!(result.is_ok());

    let st = h.get_state(&rt);
    let (dl_idx, partition_id) = st.find_sector(&rt.store, sectors_info[0].sector_number).unwrap();
    assert!(dl_idx == dest_deadline_id);

    h.check_state(&rt);

    // fault and expire

    h.declare_faults(&rt, &sectors_info);

    let st = h.get_state(&rt);
    let quant = st.quant_spec_for_deadline(rt.policy(), dl_idx);

    let current_deadline = h.current_deadline(&rt);

    let target_deadline = new_deadline_info(
        rt.policy(),
        if current_deadline.index < orig_deadline_id {
            current_deadline.period_start - rt.policy().wpost_proving_period
        } else {
            current_deadline.period_start
        },
        orig_deadline_id,
        *rt.epoch.borrow(),
    );
    let fault_expiration_epoch = target_deadline.last() + rt.policy.fault_max_age;
    let new_expiration = quant.quantize_up(fault_expiration_epoch);

    // assert that new expiration exists
    let (_, mut partition) = h.get_deadline_and_partition(&rt, dl_idx, partition_id);
    let expiration_set =
        partition.pop_expired_sectors(rt.store(), new_expiration - 1, quant).unwrap();
    assert!(expiration_set.is_empty());

    let expiration_set = partition
        .pop_expired_sectors(rt.store(), quant.quantize_up(new_expiration), quant)
        .unwrap();
    assert_eq!(expiration_set.len(), 1);
    assert!(expiration_set.early_sectors.get(sectors_info[0].sector_number));

    h.check_state(&rt);
}
