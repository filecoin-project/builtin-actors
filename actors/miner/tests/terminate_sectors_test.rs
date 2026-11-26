use fil_actor_miner::{
    Actor, CONTINUED_FAULT_PROJECTION_PERIOD, CRON_EVENT_PROCESS_EARLY_TERMINATIONS,
    CRON_EVENT_PROVING_DEADLINE, CronEventPayload, DeferredCronEventParams, ExpirationExtension2,
    ExtendSectorExpiration2Params, MaxTerminationFeeParams, MaxTerminationFeeReturn, Method,
    SectorOnChainInfo, State, TERM_FEE_MAX_FAULT_FEE_MULTIPLE_DENOM,
    TERM_FEE_MAX_FAULT_FEE_MULTIPLE_NUM, TERM_FEE_PLEDGE_MULTIPLE_DENOM,
    TERM_FEE_PLEDGE_MULTIPLE_NUM, TerminateSectorsParams, TerminationDeclaration,
    expected_reward_for_power, pledge_penalty_for_termination, power_for_sector,
    qa_power_for_sector,
};
use fil_actors_runtime::{
    BURNT_FUNDS_ACTOR_ADDR, BatchReturn, STORAGE_MARKET_ACTOR_ADDR, STORAGE_POWER_ACTOR_ADDR,
    SYSTEM_ACTOR_ADDR,
    reward::FilterEstimate,
    runtime::Runtime,
    test_utils::{ACCOUNT_ACTOR_CODE_ID, MockRuntime, expect_abort_contains_message},
};
use fvm_ipld_bitfield::BitField;
use fvm_shared::piece::PieceInfo;
use fvm_shared::{
    METHOD_SEND, MethodNum, bigint::BigInt, econ::TokenAmount, error::ExitCode,
    sector::StoragePower,
};
use std::collections::HashMap;

mod util;

use fil_actor_miner::ext::market::{
    ON_MINER_SECTORS_TERMINATE_METHOD, OnMinerSectorsTerminateParams,
};
use fil_actor_miner::ext::power::{UPDATE_PLEDGE_TOTAL_METHOD, UpdateClaimedPowerParams};
use fil_actor_reward::DENOM;
use fil_actors_runtime::test_utils::POWER_ACTOR_CODE_ID;
use fvm_ipld_encoding::RawBytes;
use fvm_ipld_encoding::ipld_block::IpldBlock;
use fvm_shared::piece::PaddedPieceSize;
use fvm_shared::sector::SectorNumber;
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
fn removes_sectors_with_correct_accounting() {
    let (mut h, rt) = setup();

    // set the reward such that our termination fees don't bottom out at the fault fee multiple and
    // instead invoke the duration multipler so we can also test that
    h.epoch_reward_smooth = FilterEstimate::new(BigInt::from(1e12 as u64), BigInt::zero());

    // 3 sectors: one left alone, one that we'll extend and one that we'll update, they should all
    // have the same termination fee
    let sectors = h.commit_and_prove_sectors(&rt, 3, DEFAULT_SECTOR_EXPIRATION, Vec::new(), true);
    assert_eq!(sectors.len(), 3);

    // advance enough to make the duration component of the termination fee large enough to not hit
    // the minimum bound
    for _ in 0..40 {
        h.advance_and_submit_posts(&rt, &sectors);
    }

    // extend the second sector, shouldn't affect the termination fee
    let state: State = rt.get_state();
    let (deadline_index, partition_index) =
        state.find_sector(rt.store(), sectors[1].sector_number).unwrap();
    let extension = 42 * rt.policy.wpost_proving_period;
    let new_expiration = sectors[1].expiration + extension;
    let params = ExtendSectorExpiration2Params {
        extensions: vec![ExpirationExtension2 {
            deadline: deadline_index,
            partition: partition_index,
            sectors: make_bitfield(&[sectors[1].sector_number]),
            sectors_with_claims: vec![],
            new_expiration,
        }],
    };
    h.extend_sectors2(&rt, params).unwrap();

    // update the third sector with no pieces, should not affect the termination fee
    let updates = make_update_manifest(&rt.get_state(), rt.store(), sectors[2].sector_number, &[]); // No pieces
    let (result, _) = h
        .prove_replica_updates3_batch(
            &rt,
            &[updates],
            true,
            true,
            ProveReplicaUpdatesConfig::default(),
        )
        .unwrap();
    assert_eq!(BatchReturn::of(&[ExitCode::OK; 1]), result.activation_results);

    // A miner will pay the minimum of termination fee and locked funds. Add some locked funds to ensure
    // correct fee calculation is used.
    h.apply_rewards(&rt, BIG_REWARDS.clone(), TokenAmount::zero());
    let state: State = rt.get_state();
    let initial_locked_funds = state.locked_funds;

    // fee for all sectors is the same, so we can just use the first one
    let expected_fee = calc_expected_fee_for_termination(&h, &rt, &sectors[0]) * 3;

    let bf = bitfield_from_slice(&sectors.iter().map(|s| s.sector_number).collect::<Vec<u64>>());
    h.terminate_sectors(&rt, &bf, expected_fee.clone());

    // expect sector to be marked as terminated and the early termination queue to be empty (having been fully processed)
    let state: State = rt.get_state();
    let (_, mut partition) = h.find_sector(&rt, sectors[0].sector_number);
    for s in &sectors {
        assert!(partition.terminated.get(s.sector_number));
    }

    let (result, _) = partition.pop_early_terminations(rt.store(), 1000).unwrap();
    assert!(result.is_empty());

    // expect fee to have been unlocked and burnt
    assert_eq!(initial_locked_funds - expected_fee, state.locked_funds);

    //expect pledge requirement to have been decremented
    assert!(state.initial_pledge.is_zero());

    h.check_state(&rt);
}

#[test]
fn removes_sector_with_without_deals() {
    let (mut h, rt) = setup();
    // One sector with no data, one with a deal, one with a verified deal
    let sectors = h.commit_and_prove_sectors_with_cfgs(
        &rt,
        3,
        DEFAULT_SECTOR_EXPIRATION,
        vec![vec![], vec![1], vec![2]],
        true,
        ProveCommitConfig {
            verify_deals_exit: Default::default(),
            claim_allocs_exit: Default::default(),
            activated_deals: HashMap::from_iter(vec![
                (1, vec![PieceInfo { size: PaddedPieceSize(1024), cid: Default::default() }]),
                (2, vec![PieceInfo { size: PaddedPieceSize(1024), cid: Default::default() }]),
            ]),
        },
    );
    let snos: Vec<SectorNumber> = sectors.iter().map(|s| s.sector_number).collect();
    // Piece spacetime is recorded in verified_deal_weight; deal_weight stays zero.
    assert!(sectors[0].verified_deal_weight.is_zero());
    assert!(!sectors[1].verified_deal_weight.is_zero());
    assert!(!sectors[2].verified_deal_weight.is_zero());
    assert!(sectors[2].deal_weight.is_zero());

    h.advance_and_submit_posts(&rt, &sectors);
    // Add locked funds to ensure correct fee calculation is used.
    h.apply_rewards(&rt, BIG_REWARDS.clone(), TokenAmount::zero());

    // Expectations about the correct call to market actor are in the harness method.
    let expected_fee: TokenAmount = sectors
        .iter()
        .fold(TokenAmount::zero(), |acc, s| acc + calc_expected_fee_for_termination(&h, &rt, s));
    h.terminate_sectors(&rt, &bitfield_from_slice(&snos), expected_fee);
    let state: State = rt.get_state();
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
    let (deadline_index, partition_index) =
        state.find_sector(rt.store(), sector.sector_number).unwrap();

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
fn owner_cannot_terminate_if_market_fails() {
    let (mut h, rt) = setup();

    let deal_ids = vec![10];
    let sector_info = h.commit_and_prove_sectors_with_cfgs(
        &rt,
        1,
        DEFAULT_SECTOR_EXPIRATION,
        vec![deal_ids],
        true,
        ProveCommitConfig {
            verify_deals_exit: Default::default(),
            claim_allocs_exit: Default::default(),
            activated_deals: HashMap::from_iter(vec![(
                0,
                vec![PieceInfo { size: PaddedPieceSize(1024), cid: Default::default() }],
            )]),
        },
    );

    assert_eq!(sector_info.len(), 1);

    h.advance_and_submit_posts(&rt, &sector_info);
    let sector = sector_info.into_iter().next().unwrap();

    let state: State = rt.get_state();
    let (deadline_index, partition_index) =
        state.find_sector(rt.store(), sector.sector_number).unwrap();

    let expected_fee = calc_expected_fee_for_termination(&h, &rt, &sector);

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

    let sectors_bf = BitField::try_from_bits([sector.sector_number]).unwrap();
    rt.expect_send_simple(
        STORAGE_MARKET_ACTOR_ADDR,
        ON_MINER_SECTORS_TERMINATE_METHOD,
        IpldBlock::serialize_cbor(&OnMinerSectorsTerminateParams {
            epoch: *rt.epoch.borrow(),
            sectors: sectors_bf,
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

    assert!(
        rt.call::<Actor>(
            Method::OnDeferredCronEvent as u64,
            IpldBlock::serialize_cbor(&DeferredCronEventParams {
                event_payload: Vec::from(RawBytes::serialize(payload).unwrap().bytes()),
                reward_smoothed: h.epoch_reward_smooth.clone(),
                quality_adj_power_smoothed: h.epoch_qa_power_smooth.clone(),
                implicit_stream_weight: h.implicit_stream_weight,
            })
            .unwrap(),
        )
        .unwrap()
        .is_none()
    );

    rt.verify();

    h.check_state(&rt);
}

fn calc_expected_fee_for_termination(
    h: &ActorHarness,
    rt: &MockRuntime,
    sector: &SectorOnChainInfo,
) -> TokenAmount {
    let sector_power = qa_power_for_sector(sector.seal_proof.sector_size().unwrap(), sector);
    let sector_age = *rt.epoch.borrow() - sector.activation;
    let initial_pledge = &sector.initial_pledge;
    let fault_fee = expected_scaled_fault_fee(h, &sector_power);
    pledge_penalty_for_termination(initial_pledge, sector_age, &fault_fee)
}

#[test]
fn max_termination_fee_returns_correct_results() {
    let (mut h, rt) = setup();

    let deal_ids = vec![10];
    let sector_info =
        h.commit_and_prove_sectors(&rt, 1, DEFAULT_SECTOR_EXPIRATION, vec![deal_ids], true);
    assert_eq!(sector_info.len(), 1);

    h.advance_and_submit_posts(&rt, &sector_info);

    let sector = sector_info.into_iter().next().unwrap();
    let initial_pledge = sector.initial_pledge.clone();
    let qa_sector_power = power_for_sector(sector.seal_proof.sector_size().unwrap(), &sector).qa;

    let pledge_floor = (initial_pledge.clone() * TERM_FEE_PLEDGE_MULTIPLE_NUM)
        .div_floor(TERM_FEE_PLEDGE_MULTIPLE_DENOM);
    for weight in [DENOM, DENOM / 2, 0] {
        h.implicit_stream_weight = weight;
        let fault_fee_bound = (expected_scaled_fault_fee(&h, &qa_sector_power)
            * TERM_FEE_MAX_FAULT_FEE_MULTIPLE_NUM)
            .div_floor(TERM_FEE_MAX_FAULT_FEE_MULTIPLE_DENOM);
        if weight > 0 {
            assert!(fault_fee_bound > pledge_floor);
        } else {
            assert!(fault_fee_bound.is_zero());
        }
        let cases = [
            // With no power, only the pledge-based maximum remains.
            (StoragePower::zero(), pledge_floor.clone()),
            // The fault-fee bound scales, while the pledge-based floor stays fixed.
            (qa_sector_power.clone(), std::cmp::max(pledge_floor.clone(), fault_fee_bound)),
        ];

        for (quality_adj_power, expected) in cases {
            let params = MaxTerminationFeeParams {
                initial_pledge: initial_pledge.clone(),
                power: quality_adj_power,
            };

            h.expect_query_network_info(&rt);
            rt.expect_validate_caller_any();
            let res = rt
                .call::<Actor>(
                    Method::MaxTerminationFeeExported as MethodNum,
                    IpldBlock::serialize_cbor(&params).unwrap(),
                )
                .unwrap()
                .unwrap()
                .deserialize::<MaxTerminationFeeReturn>()
                .unwrap();
            rt.verify();

            assert_eq!(expected, res.max_fee, "implicit stream weight {weight}");
        }
    }

    h.check_state(&rt);
}

// Calculate the expected scaling independently of the weighted fault-penalty helper.
fn expected_scaled_fault_fee(h: &ActorHarness, qa_power: &StoragePower) -> TokenAmount {
    let unscaled = expected_reward_for_power(
        &h.epoch_reward_smooth,
        &h.epoch_qa_power_smooth,
        qa_power,
        CONTINUED_FAULT_PROJECTION_PERIOD,
    );
    (unscaled * h.implicit_stream_weight).div_floor(DENOM)
}

#[test]
fn direct_termination_uses_implicit_stream_weight() {
    for weight in [DENOM, DENOM / 2, 0] {
        let (mut h, rt) = setup();
        let sectors =
            h.commit_and_prove_sectors(&rt, 1, DEFAULT_SECTOR_EXPIRATION, Vec::new(), true);
        h.advance_and_submit_posts(&rt, &sectors);
        h.apply_rewards(&rt, BIG_REWARDS.clone(), TokenAmount::zero());
        h.implicit_stream_weight = weight;

        let before = h.get_state(&rt);
        let expected = calc_expected_fee_for_termination(&h, &rt, &sectors[0]);
        assert!(expected.is_positive()); // Pledge-based fees remain even at zero weight.
        let pledge_only_fee = pledge_penalty_for_termination(
            &sectors[0].initial_pledge,
            *rt.epoch.borrow() - sectors[0].activation,
            &TokenAmount::zero(),
        );
        if weight > 0 {
            assert!(expected > pledge_only_fee, "fault fee must determine the termination fee");
        } else {
            assert_eq!(expected, pledge_only_fee);
        }
        h.terminate_sectors(
            &rt,
            &bitfield_from_slice(&[sectors[0].sector_number]),
            expected.clone(),
        );
        let after = h.get_state(&rt);
        assert_eq!(before.locked_funds - expected, after.locked_funds);
        assert!(after.initial_pledge.is_zero());
        assert!(after.early_terminations.is_empty());
        h.check_state(&rt);
    }
}

#[test]
fn termination_cron_uses_implicit_stream_weight() {
    for from_proving_deadline in [false, true] {
        for weight in [DENOM, DENOM / 2, 0] {
            let (mut h, mut rt) = setup();
            let sectors =
                h.commit_and_prove_sectors(&rt, 1, DEFAULT_SECTOR_EXPIRATION, Vec::new(), true);
            h.advance_and_submit_posts(&rt, &sectors);
            let sector = &sectors[0];
            let (deadline_index, _) =
                h.get_state(&rt).find_sector(rt.store(), sector.sector_number).unwrap();
            let deadline = h.advance_to_deadline(&rt, deadline_index);
            rt.set_epoch(deadline.last());
            h.apply_rewards(&rt, BIG_REWARDS.clone(), TokenAmount::zero());
            h.implicit_stream_weight = weight;

            // Expire a newly detected fault at this deadline, so cron must process a
            // real early termination without waiting through the fault maximum age.
            rt.policy.fault_max_age = 0;
            let before = h.get_state(&rt);
            assert!(before.early_terminations.is_empty());
            let expected_fee = calc_expected_fee_for_termination(&h, &rt, sector);
            assert!(expected_fee.is_positive());
            let pledge_only_fee = pledge_penalty_for_termination(
                &sector.initial_pledge,
                *rt.epoch.borrow() - sector.activation,
                &TokenAmount::zero(),
            );
            if weight > 0 {
                assert!(
                    expected_fee > pledge_only_fee,
                    "fault fee must determine the termination fee"
                );
            } else {
                assert_eq!(expected_fee, pledge_only_fee);
            }

            if !from_proving_deadline {
                // Seed the deferred callback with exactly the state produced by deadline
                // expiration. Retain the pledge until the callback charges its penalty.
                let mut state = before.clone();
                let expired =
                    state.advance_deadline(&rt.policy, rt.store(), *rt.epoch.borrow()).unwrap();
                assert!(expired.previously_faulty_power.qa.is_zero());
                assert!(expired.daily_fee.is_zero());
                assert!(expired.pledge_delta.is_zero());
                assert_eq!(before.initial_pledge, state.initial_pledge);
                assert!(state.early_terminations.get(deadline_index));
                rt.replace_state(&state);
            }

            rt.expect_validate_caller_addr(vec![STORAGE_POWER_ACTOR_ADDR]);
            if from_proving_deadline {
                // Newly detected faults lose power but incur no continued-fault fee.
                // The subsequent burn therefore comes from nested termination processing.
                let power = power_for_sector(h.sector_size, sector);
                rt.expect_send_simple(
                    STORAGE_POWER_ACTOR_ADDR,
                    fil_actor_power::Method::UpdateClaimedPower as u64,
                    IpldBlock::serialize_cbor(&UpdateClaimedPowerParams {
                        raw_byte_delta: -power.raw,
                        quality_adjusted_delta: -power.qa,
                    })
                    .unwrap(),
                    TokenAmount::zero(),
                    None,
                    ExitCode::OK,
                );
                rt.expect_send_simple(
                    STORAGE_POWER_ACTOR_ADDR,
                    fil_actor_power::Method::EnrollCronEvent as u64,
                    IpldBlock::serialize_cbor(&make_deadline_cron_event_params(
                        deadline.last() + rt.policy.wpost_challenge_window,
                    ))
                    .unwrap(),
                    TokenAmount::zero(),
                    None,
                    ExitCode::OK,
                );
            }
            rt.expect_send_simple(
                BURNT_FUNDS_ACTOR_ADDR,
                METHOD_SEND,
                None,
                expected_fee.clone(),
                None,
                ExitCode::OK,
            );
            rt.expect_send_simple(
                STORAGE_POWER_ACTOR_ADDR,
                UPDATE_PLEDGE_TOTAL_METHOD,
                IpldBlock::serialize_cbor(&(-sector.initial_pledge.clone() - &expected_fee))
                    .unwrap(),
                TokenAmount::zero(),
                None,
                ExitCode::OK,
            );
            expect_event(&rt, "sector-terminated", &sector.sector_number);

            rt.set_origin(SYSTEM_ACTOR_ADDR);
            rt.set_caller(*POWER_ACTOR_CODE_ID, STORAGE_POWER_ACTOR_ADDR);
            let payload = CronEventPayload {
                event_type: if from_proving_deadline {
                    CRON_EVENT_PROVING_DEADLINE
                } else {
                    CRON_EVENT_PROCESS_EARLY_TERMINATIONS
                },
            };
            let params = DeferredCronEventParams {
                event_payload: Vec::from(RawBytes::serialize(payload).unwrap().bytes()),
                reward_smoothed: h.epoch_reward_smooth.clone(),
                quality_adj_power_smoothed: h.epoch_qa_power_smooth.clone(),
                implicit_stream_weight: weight,
            };
            assert!(
                rt.call::<Actor>(
                    Method::OnDeferredCronEvent as u64,
                    IpldBlock::serialize_cbor(&params).unwrap(),
                )
                .unwrap()
                .is_none()
            );
            rt.verify();

            let after = h.get_state(&rt);
            assert_eq!(before.locked_funds - expected_fee, after.locked_funds);
            assert!(after.initial_pledge.is_zero());
            assert!(after.early_terminations.is_empty());
            assert!(
                h.find_sector(&rt, sector.sector_number).1.terminated.get(sector.sector_number)
            );
            h.check_state(&rt);
        }
    }
}
