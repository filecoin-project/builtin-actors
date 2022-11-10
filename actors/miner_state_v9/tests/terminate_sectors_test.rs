use fil_actor_miner_state_v9::{
    expected_reward_for_power, pledge_penalty_for_termination, qa_power_for_sector, Actor, Method,
    State, TerminateSectorsParams, TerminationDeclaration, INITIAL_PLEDGE_PROJECTION_PERIOD,
};
use fil_actors_runtime_common::{
    runtime::{Policy, Runtime},
    test_utils::{expect_abort_contains_message, MockRuntime, ACCOUNT_ACTOR_CODE_ID},
    EPOCHS_IN_DAY,
};
use fvm_ipld_encoding::RawBytes;
use fvm_shared::{econ::TokenAmount, error::ExitCode};

mod util;

use num_traits::Zero;
use util::*;

fn setup() -> (ActorHarness, MockRuntime) {
    let big_balance = 20u128.pow(23);
    let period_offset = 100;
    let precommit_epoch = 1;

    let h = ActorHarness::new(period_offset);
    let mut rt = h.new_runtime();
    h.construct_and_verify(&mut rt);
    rt.balance.replace(TokenAmount::from_atto(big_balance));
    rt.set_epoch(precommit_epoch);

    (h, rt)
}

#[test]
fn removes_sector_with_correct_accounting() {
    let (mut h, mut rt) = setup();

    let sector_info =
        h.commit_and_prove_sectors(&mut rt, 1, DEFAULT_SECTOR_EXPIRATION, Vec::new(), true);

    assert_eq!(sector_info.len(), 1);
    h.advance_and_submit_posts(&mut rt, &sector_info);
    let sector = sector_info.into_iter().next().unwrap();

    // A miner will pay the minimum of termination fee and locked funds. Add some locked funds to ensure
    // correct fee calculation is used.
    h.apply_rewards(&mut rt, BIG_REWARDS.clone(), TokenAmount::zero());
    let state: State = rt.get_state();
    let initial_locked_funds = state.locked_funds;

    let sector_size = sector.seal_proof.sector_size().unwrap();
    let sector_power = qa_power_for_sector(sector_size, &sector);
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
    let sector_age = rt.epoch - sector.activation;
    let expected_fee = pledge_penalty_for_termination(
        &day_reward,
        sector_age,
        &twenty_day_reward,
        &h.epoch_qa_power_smooth,
        &sector_power,
        &h.epoch_reward_smooth,
        &TokenAmount::zero(),
        0,
    );

    let sectors = bitfield_from_slice(&[sector.sector_number]);
    h.terminate_sectors(&mut rt, &sectors, expected_fee.clone());

    // expect sector to be marked as terminated and the early termination queue to be empty (having been fully processed)
    let state: State = rt.get_state();
    let (_, mut partition) = h.find_sector(&rt, sector.sector_number);
    let terminated = partition.terminated.get(sector.sector_number);
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
fn cannot_terminate_a_sector_when_the_challenge_window_is_open() {
    let (mut h, mut rt) = setup();

    let sector_info =
        h.commit_and_prove_sectors(&mut rt, 1, DEFAULT_SECTOR_EXPIRATION, Vec::new(), true);

    assert_eq!(sector_info.len(), 1);
    h.advance_and_submit_posts(&mut rt, &sector_info);
    let sector = sector_info.into_iter().next().unwrap();

    let state: State = rt.get_state();
    let policy = Policy::default();
    let (deadline_index, partition_index) =
        state.find_sector(&policy, rt.store(), sector.sector_number).unwrap();

    // advance into the deadline but not past it
    h.advance_to_deadline(&mut rt, deadline_index);

    let params = TerminateSectorsParams {
        terminations: vec![TerminationDeclaration {
            deadline: deadline_index,
            partition: partition_index,
            sectors: util::make_bitfield(&[sector.sector_number]),
        }],
    };

    rt.set_caller(*ACCOUNT_ACTOR_CODE_ID, h.worker);
    rt.expect_validate_caller_addr(h.caller_addrs());
    let res =
        rt.call::<Actor>(Method::TerminateSectors as u64, &RawBytes::serialize(params).unwrap());
    expect_abort_contains_message(
        ExitCode::USR_ILLEGAL_ARGUMENT,
        "cannot terminate sectors in immutable deadline",
        res,
    );

    h.check_state(&rt);
}
