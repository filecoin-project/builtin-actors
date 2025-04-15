use fil_actor_miner::{
    Actor, CRON_EVENT_PROCESS_EARLY_TERMINATIONS, CronEventPayload, DeferredCronEventParams,
    ExpirationExtension2, ExtendSectorExpiration2Params, MaxTerminationFeeParams,
    MaxTerminationFeeReturn, Method, SectorOnChainInfo, State,
    TERM_FEE_MAX_FAULT_FEE_MULTIPLE_DENOM, TERM_FEE_MAX_FAULT_FEE_MULTIPLE_NUM,
    TERM_FEE_PLEDGE_MULTIPLE_DENOM, TERM_FEE_PLEDGE_MULTIPLE_NUM, TerminateSectorsParams,
    TerminationDeclaration, pledge_penalty_for_continued_fault, pledge_penalty_for_termination,
    power_for_sector, qa_power_for_sector,
};
use fil_actors_runtime::{
    BURNT_FUNDS_ACTOR_ADDR, BatchReturn, STORAGE_MARKET_ACTOR_ADDR, STORAGE_POWER_ACTOR_ADDR,
    SYSTEM_ACTOR_ADDR,
    reward::FilterEstimate,
    runtime::Runtime,
    test_utils::{ACCOUNT_ACTOR_CODE_ID, MockRuntime, expect_abort_contains_message},
};
use fvm_ipld_bitfield::BitField;
use fvm_shared::{
    METHOD_SEND, MethodNum, bigint::BigInt, econ::TokenAmount, error::ExitCode,
    sector::StoragePower,
};
use std::collections::HashMap;

mod util;

use fil_actor_market::{ActivatedDeal, NO_ALLOCATION_ID};
use fil_actor_miner::ext::market::{
    ON_MINER_SECTORS_TERMINATE_METHOD, OnMinerSectorsTerminateParams,
};
use fil_actor_miner::ext::power::UPDATE_PLEDGE_TOTAL_METHOD;
use fil_actors_runtime::test_utils::POWER_ACTOR_CODE_ID;
use fvm_ipld_encoding::RawBytes;
use fvm_ipld_encoding::ipld_block::IpldBlock;
use fvm_shared::piece::PaddedPieceSize;
use fvm_shared::sector::SectorNumber;
use num_traits::{Signed, Zero};
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
    h.extend_sectors2(&rt, params, HashMap::new()).unwrap();

    // update the third sector with no pieces, should not affect the termination fee
    let updates = make_update_manifest(&rt.get_state(), rt.store(), sectors[2].sector_number, &[]); // No pieces
    let (result, _, _) = h
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
                (
                    1,
                    vec![ActivatedDeal {
                        client: 0,
                        allocation_id: NO_ALLOCATION_ID,
                        data: Default::default(),
                        size: PaddedPieceSize(1024),
                    }],
                ),
                (
                    2,
                    vec![ActivatedDeal {
                        client: 0,
                        allocation_id: 1,
                        data: Default::default(),
                        size: PaddedPieceSize(1024),
                    }],
                ),
            ]),
        },
    );
    let snos: Vec<SectorNumber> = sectors.iter().map(|s| s.sector_number).collect();
    assert!(sectors[0].deal_weight.is_zero());
    assert!(sectors[1].deal_weight.is_positive());
    assert!(sectors[2].verified_deal_weight.is_positive());

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
                vec![ActivatedDeal {
                    client: 0,
                    allocation_id: NO_ALLOCATION_ID,
                    data: Default::default(),
                    size: PaddedPieceSize(1024),
                }],
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
    let fault_fee = pledge_penalty_for_continued_fault(
        &h.epoch_reward_smooth,
        &h.epoch_qa_power_smooth,
        &sector_power,
    );
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

    let cases = [
        // low power resulting in low fault fee => termination fee should be pledge multiple *
        // initial pledge
        (
            StoragePower::zero(),
            (initial_pledge.clone() * TERM_FEE_PLEDGE_MULTIPLE_NUM)
                .div_floor(TERM_FEE_PLEDGE_MULTIPLE_DENOM),
        ),
        // high power resulting in high fault fee => termination fee should be fault fee * max
        // fault fee multiple
        (
            qa_sector_power.clone(),
            (pledge_penalty_for_continued_fault(
                &h.epoch_reward_smooth,
                &h.epoch_qa_power_smooth,
                &qa_sector_power,
            ) * TERM_FEE_MAX_FAULT_FEE_MULTIPLE_NUM)
                .div_floor(TERM_FEE_MAX_FAULT_FEE_MULTIPLE_DENOM),
        ),
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

        assert_eq!(expected, res.max_fee);
    }

    h.check_state(&rt);
}
