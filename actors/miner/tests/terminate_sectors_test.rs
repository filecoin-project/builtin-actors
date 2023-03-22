use fil_actor_miner::{
    expected_reward_for_power, pledge_penalty_for_termination, qa_power_for_sector, Actor,
    CronEventPayload, DeferredCronEventParams, Method, SectorOnChainInfo, State,
    TerminateSectorsParams, TerminationDeclaration, CRON_EVENT_PROCESS_EARLY_TERMINATIONS,
    INITIAL_PLEDGE_PROJECTION_PERIOD,
};
use fil_actors_runtime::{
    runtime::{Policy, Runtime},
    test_utils::{expect_abort_contains_message, MockRuntime, ACCOUNT_ACTOR_CODE_ID},
    BURNT_FUNDS_ACTOR_ADDR, EPOCHS_IN_DAY, STORAGE_MARKET_ACTOR_ADDR, STORAGE_POWER_ACTOR_ADDR,
    SYSTEM_ACTOR_ADDR,
};
use fvm_shared::{econ::TokenAmount, error::ExitCode, METHOD_SEND};

mod util;

use fil_actor_miner::ext::market::{
    OnMinerSectorsTerminateParams, ON_MINER_SECTORS_TERMINATE_METHOD,
};
use fil_actor_miner::ext::power::UPDATE_PLEDGE_TOTAL_METHOD;
use fil_actors_runtime::test_utils::POWER_ACTOR_CODE_ID;
use fvm_ipld_encoding::ipld_block::IpldBlock;
use fvm_ipld_encoding::RawBytes;
use num_traits::Zero;
use util::*;

fn setup() -> (ActorHarness, MockRuntime) {
    let big_balance = 20u128.pow(23);
    let period_offset = 100;
    let precommit_epoch = 1;

    let h = ActorHarness::new(period_offset);
    let rt = h.new_runtime();
    h.construct_and_verify(&rt);
    rt.balance.replace(TokenAmount::from_atto(big_balance));
    rt.set_epoch(precommit_epoch);

    (h, rt)
}

#[test]
fn removes_sector_with_correct_accounting() {
    let (mut h, rt) = setup();

    let sector_info =
        h.commit_and_prove_sectors(&rt, 1, DEFAULT_SECTOR_EXPIRATION, Vec::new(), true);

    assert_eq!(sector_info.len(), 1);
    h.advance_and_submit_posts(&rt, &sector_info);
    let sector = sector_info.into_iter().next().unwrap();

    // A miner will pay the minimum of termination fee and locked funds. Add some locked funds to ensure
    // correct fee calculation is used.
    h.apply_rewards(&rt, BIG_REWARDS.clone(), TokenAmount::zero());
    let state: State = rt.get_state();
    let initial_locked_funds = state.locked_funds;

    let expected_fee = calc_expected_fee_for_termination(&h, &rt, sector.clone());

    let sectors = bitfield_from_slice(&[sector.sector_number]);
    h.terminate_sectors(&rt, &sectors, expected_fee.clone());

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
    let (mut h, rt) = setup();

    let sector_info =
        h.commit_and_prove_sectors(&rt, 1, DEFAULT_SECTOR_EXPIRATION, Vec::new(), true);

    assert_eq!(sector_info.len(), 1);
    h.advance_and_submit_posts(&rt, &sector_info);
    let sector = sector_info.into_iter().next().unwrap();

    let state: State = rt.get_state();
    let policy = Policy::default();
    let (deadline_index, partition_index) =
        state.find_sector(&policy, rt.store(), sector.sector_number).unwrap();

    // advance into the deadline but not past it
    h.advance_to_deadline(&rt, deadline_index);

    let params = TerminateSectorsParams {
        terminations: vec![TerminationDeclaration {
            deadline: deadline_index,
            partition: partition_index,
            sectors: util::make_bitfield(&[sector.sector_number]),
        }],
    };

    rt.set_caller(*ACCOUNT_ACTOR_CODE_ID, h.worker);
    rt.expect_validate_caller_addr(h.caller_addrs());
    let res = rt.call::<Actor>(
        Method::TerminateSectors as u64,
        IpldBlock::serialize_cbor(&params).unwrap(),
    );
    expect_abort_contains_message(
        ExitCode::USR_ILLEGAL_ARGUMENT,
        "cannot terminate sectors in immutable deadline",
        res,
    );

    h.check_state(&rt);
}

#[test]
fn owner_cannot_terminate_if_market_cron_fails() {
    let (mut h, rt) = setup();

    let deal_ids = vec![10];
    let sector_info =
        h.commit_and_prove_sectors(&rt, 1, DEFAULT_SECTOR_EXPIRATION, vec![deal_ids.clone()], true);

    assert_eq!(sector_info.len(), 1);

    h.advance_and_submit_posts(&rt, &sector_info);
    let sector = sector_info.into_iter().next().unwrap();

    let state: State = rt.get_state();
    let policy = Policy::default();
    let (deadline_index, partition_index) =
        state.find_sector(&policy, rt.store(), sector.sector_number).unwrap();

    let expected_fee = calc_expected_fee_for_termination(&h, &rt, sector.clone());

    rt.expect_validate_caller_addr(h.caller_addrs());

    h.expect_query_network_info(&rt);
    rt.expect_send_simple(
        BURNT_FUNDS_ACTOR_ADDR,
        METHOD_SEND,
        None,
        expected_fee,
        None,
        ExitCode::OK,
    );

    rt.expect_send_simple(
        STORAGE_POWER_ACTOR_ADDR,
        UPDATE_PLEDGE_TOTAL_METHOD,
        IpldBlock::serialize_cbor(&(-sector.clone().initial_pledge)).unwrap(),
        TokenAmount::zero(),
        None,
        ExitCode::OK,
    );

    rt.expect_send_simple(
        STORAGE_MARKET_ACTOR_ADDR,
        ON_MINER_SECTORS_TERMINATE_METHOD,
        IpldBlock::serialize_cbor(&OnMinerSectorsTerminateParams {
            epoch: *rt.epoch.borrow(),
            deal_ids,
        })
        .unwrap(),
        TokenAmount::zero(),
        None,
        ExitCode::USR_ILLEGAL_STATE,
    );

    rt.set_origin(h.worker);
    rt.set_caller(*ACCOUNT_ACTOR_CODE_ID, h.worker);
    assert_eq!(
        ExitCode::USR_ILLEGAL_STATE,
        rt.call::<Actor>(
            Method::TerminateSectors as u64,
            IpldBlock::serialize_cbor(&TerminateSectorsParams {
                terminations: vec![TerminationDeclaration {
                    deadline: deadline_index,
                    partition: partition_index,
                    sectors: make_bitfield(&[sector.sector_number]),
                }],
            })
            .unwrap(),
        )
        .unwrap_err()
        .exit_code()
    );

    rt.verify();

    h.check_state(&rt);
}

#[test]
fn system_can_terminate_if_market_cron_fails() {
    let (mut h, rt) = setup();

    let deal_ids = vec![10];
    let sector_info =
        h.commit_and_prove_sectors(&rt, 1, DEFAULT_SECTOR_EXPIRATION, vec![deal_ids], true);

    assert_eq!(sector_info.len(), 1);

    h.advance_and_submit_posts(&rt, &sector_info);
    rt.expect_validate_caller_addr(vec![STORAGE_POWER_ACTOR_ADDR]);

    rt.set_origin(SYSTEM_ACTOR_ADDR);
    rt.set_caller(*POWER_ACTOR_CODE_ID, STORAGE_POWER_ACTOR_ADDR);
    let payload = CronEventPayload { event_type: CRON_EVENT_PROCESS_EARLY_TERMINATIONS };

    assert!(rt
        .call::<Actor>(
            Method::OnDeferredCronEvent as u64,
            IpldBlock::serialize_cbor(&DeferredCronEventParams {
                event_payload: Vec::from(RawBytes::serialize(payload).unwrap().bytes()),
                reward_smoothed: h.epoch_reward_smooth.clone(),
                quality_adj_power_smoothed: h.epoch_qa_power_smooth.clone(),
            })
            .unwrap(),
        )
        .unwrap()
        .is_none());

    rt.verify();

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
