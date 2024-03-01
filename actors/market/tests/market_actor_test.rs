// Copyright 2019-2022 ChainSafe Systems
// SPDX-License-Identifier: Apache-2.0, MIT

use std::cell::RefCell;
use std::ops::Add;

use frc46_token::token::types::{TransferFromParams, TransferFromReturn};
use fvm_ipld_amt::Amt;
use fvm_ipld_encoding::ipld_block::IpldBlock;
use fvm_ipld_encoding::{to_vec, RawBytes};
use fvm_shared::address::Address;
use fvm_shared::clock::{ChainEpoch, EPOCH_UNDEFINED};
use fvm_shared::crypto::signature::Signature;
use fvm_shared::deal::DealID;
use fvm_shared::econ::TokenAmount;
use fvm_shared::error::ExitCode;
use fvm_shared::piece::PaddedPieceSize;
use fvm_shared::sector::{RegisteredSealProof, StoragePower};
use fvm_shared::sys::SendFlags;
use fvm_shared::{MethodNum, METHOD_CONSTRUCTOR, METHOD_SEND};
use num_traits::{FromPrimitive, Zero};
use regex::Regex;

use fil_actor_market::balance_table::BalanceTable;
use fil_actor_market::ext::account::{AuthenticateMessageParams, AUTHENTICATE_MESSAGE_METHOD};
use fil_actor_market::ext::verifreg::{AllocationRequest, AllocationsResponse};
use fil_actor_market::policy::detail::DEAL_MAX_LABEL_SIZE;
use fil_actor_market::{
    ext, Actor as MarketActor, BatchActivateDealsResult, ClientDealProposal, DealArray,
    DealMetaArray, DealOpsByEpoch, Label, MarketNotifyDealParams, Method,
    PendingDealAllocationsMap, PendingProposalsSet, PublishStorageDealsParams,
    PublishStorageDealsReturn, SectorDeals, State, WithdrawBalanceParams, DEAL_OPS_BY_EPOCH_CONFIG,
    EX_DEAL_EXPIRED, MARKET_NOTIFY_DEAL_METHOD, PENDING_ALLOCATIONS_CONFIG,
    PENDING_PROPOSALS_CONFIG, PROPOSALS_AMT_BITWIDTH, STATES_AMT_BITWIDTH,
};
use fil_actors_runtime::cbor::{deserialize, serialize};
use fil_actors_runtime::network::EPOCHS_IN_DAY;
use fil_actors_runtime::runtime::{Policy, Runtime};
use fil_actors_runtime::test_utils::*;
use fil_actors_runtime::{
    ActorError, BatchReturn, SetMultimap, SetMultimapConfig, BURNT_FUNDS_ACTOR_ADDR,
    DATACAP_TOKEN_ACTOR_ADDR, DEFAULT_HAMT_CONFIG, SYSTEM_ACTOR_ADDR, VERIFIED_REGISTRY_ACTOR_ADDR,
};
use harness::*;

mod harness;

#[test]
fn test_remove_all_error() {
    let market_actor = Address::new_id(100);
    let rt = MockRuntime { receiver: market_actor, ..Default::default() };
    let config = SetMultimapConfig { outer: DEFAULT_HAMT_CONFIG, inner: DEFAULT_HAMT_CONFIG };
    SetMultimap::<_, u64, u64>::empty(&rt.store(), config, "test")
        .remove_all(&42u64)
        .expect("expected no error");
}

#[test]
fn simple_construction() {
    let rt = MockRuntime {
        receiver: Address::new_id(100),
        caller: RefCell::new(SYSTEM_ACTOR_ADDR),
        caller_type: RefCell::new(*INIT_ACTOR_CODE_ID),
        ..Default::default()
    };

    rt.set_caller(*SYSTEM_ACTOR_CODE_ID, SYSTEM_ACTOR_ADDR);
    rt.expect_validate_caller_addr(vec![SYSTEM_ACTOR_ADDR]);

    assert!(rt.call::<MarketActor>(METHOD_CONSTRUCTOR, None).unwrap().is_none());

    rt.verify();

    let store = &rt.store;

    let empty_balance_table = BalanceTable::new(store, "empty").root().unwrap();
    let empty_pending_proposals =
        PendingProposalsSet::empty(store, PENDING_PROPOSALS_CONFIG, "empty").flush().unwrap();
    let empty_proposals_array =
        Amt::<(), _>::new_with_bit_width(store, PROPOSALS_AMT_BITWIDTH).flush().unwrap();
    let empty_states_array =
        Amt::<(), _>::new_with_bit_width(store, STATES_AMT_BITWIDTH).flush().unwrap();
    let empty_deal_ops =
        DealOpsByEpoch::empty(store, DEAL_OPS_BY_EPOCH_CONFIG, "empty").flush().unwrap();

    let state_data: State = rt.get_state();

    assert_eq!(empty_proposals_array, state_data.proposals);
    assert_eq!(empty_states_array, state_data.states);
    assert_eq!(empty_pending_proposals, state_data.pending_proposals);
    assert_eq!(empty_balance_table, state_data.escrow_table);
    assert_eq!(empty_balance_table, state_data.locked_table);
    assert_eq!(0, state_data.next_id);
    assert_eq!(empty_deal_ops, state_data.deal_ops_by_epoch);
    assert_eq!(state_data.last_cron, EPOCH_UNDEFINED);
}

#[test]
fn label_cbor() {
    let label = Label::String("i_am_random_string____i_am_random_string____".parse().unwrap());
    let _ = to_vec(&label)
        .map_err(|e| ActorError::from(e).wrap("failed to serialize DealProposal"))
        .unwrap();

    let label2 = Label::Bytes(b"i_am_random_____i_am_random_____".to_vec());
    let _ = to_vec(&label2)
        .map_err(|e| ActorError::from(e).wrap("failed to serialize DealProposal"))
        .unwrap();

    let empty_string_label = Label::String("".parse().unwrap());
    let sv_bz = to_vec(&empty_string_label).unwrap();
    assert_eq!(vec![0x60], sv_bz);

    let empty_bytes_label = Label::Bytes(b"".to_vec());
    let sv_bz = to_vec(&empty_bytes_label).unwrap();
    assert_eq!(vec![0x40], sv_bz);
}

#[test]
fn label_from_cbor() {
    // empty string, b001_00000
    let empty_cbor_text = vec![0x60];
    let label1: Label = deserialize(&RawBytes::from(empty_cbor_text), "empty cbor string").unwrap();
    if let Label::String(s) = label1 {
        assert_eq!("", s)
    } else {
        panic!("expected string label not bytes")
    }

    // valid utf8 string b011_01000 "deadbeef"
    let end_valid_cbor_text = b"deadbeef".to_vec();
    let mut valid_cbor_text = vec![0x68];
    for i in end_valid_cbor_text {
        valid_cbor_text.push(i);
    }
    let label2: Label = deserialize(&RawBytes::from(valid_cbor_text), "valid cbor string").unwrap();
    if let Label::String(s) = label2 {
        assert_eq!("deadbeef", s)
    } else {
        panic!("expected string label not bytes")
    }

    // invalid utf8 string 0b011_00100 0xde 0xad 0xbe 0xeef
    let invalid_cbor_text = vec![0x64, 0xde, 0xad, 0xbe, 0xef];
    let out = deserialize::<Label>(&RawBytes::from(invalid_cbor_text), "invalid cbor string");
    out.expect_err("invalid utf8 string in maj typ 3 should fail deser");

    // empty bytes, b010_00000
    let empty_cbor_bytes = vec![0x40];
    let label3: Label = deserialize(&RawBytes::from(empty_cbor_bytes), "empty cbor bytes").unwrap();
    if let Label::Bytes(b) = label3 {
        assert_eq!(Vec::<u8>::new(), b)
    } else {
        panic!("expected bytes label not string")
    }

    // bytes b010_00100 0xde 0xad 0xbe 0xef
    let cbor_bytes = vec![0x44, 0xde, 0xad, 0xbe, 0xef];
    let label4: Label = deserialize(&RawBytes::from(cbor_bytes), "cbor bytes").unwrap();
    if let Label::Bytes(b) = label4 {
        assert_eq!(vec![0xde, 0xad, 0xbe, 0xef], b)
    } else {
        panic!("expected bytes label not string")
    }

    // bad major type, array of empty array b100_00001 b100_00000
    let bad_bytes = vec![0x81, 0x80];
    let out = deserialize::<Label>(&RawBytes::from(bad_bytes), "cbor array, unexpected major type");
    out.expect_err("major type 4 should not be recognized by union type and deser should fail");
}

#[test]
fn label_non_utf8() {
    let bad_str_bytes = vec![0xde, 0xad, 0xbe, 0xef];
    let bad_str = unsafe { std::str::from_utf8_unchecked(&bad_str_bytes) };
    let bad_label = Label::String(bad_str.parse().unwrap());
    let bad_label_ser = to_vec(&bad_label)
        .map_err(|e| ActorError::from(e).wrap("failed to serialize DealProposal"))
        .unwrap();
    let out: Result<Label, _> = deserialize(&RawBytes::from(bad_label_ser), "invalid cbor string");
    out.expect_err("invalid cbor string shouldn't deser");
}

#[test]
fn adds_to_provider_escrow_funds() {
    struct TestCase {
        delta: u64,
        total: u64,
    }
    let test_cases = [
        TestCase { delta: 10, total: 10 },
        TestCase { delta: 20, total: 30 },
        TestCase { delta: 40, total: 70 },
    ];

    for caller_addr in &[OWNER_ADDR, WORKER_ADDR] {
        let rt = setup();

        for tc in &test_cases {
            rt.set_caller(*ACCOUNT_ACTOR_CODE_ID, *caller_addr);
            rt.set_received(TokenAmount::from_atto(tc.delta));
            rt.expect_validate_caller_any();
            expect_provider_control_address(&rt, PROVIDER_ADDR, OWNER_ADDR, WORKER_ADDR);

            assert!(rt
                .call::<MarketActor>(
                    Method::AddBalance as u64,
                    IpldBlock::serialize_cbor(&PROVIDER_ADDR).unwrap(),
                )
                .unwrap()
                .is_none());

            rt.verify();

            let acct = get_balance(&rt, &PROVIDER_ADDR);
            assert_eq!(acct.balance, TokenAmount::from_atto(tc.total));
            assert_eq!(acct.locked, TokenAmount::zero());
            check_state(&rt);
        }
    }
}

#[test]
fn fails_if_withdraw_from_non_provider_funds_is_not_initiated_by_the_recipient() {
    let rt = setup();

    add_participant_funds(&rt, CLIENT_ADDR, TokenAmount::from_atto(20u8));

    assert_eq!(TokenAmount::from_atto(20u8), get_balance(&rt, &CLIENT_ADDR).balance);

    rt.expect_validate_caller_addr(vec![CLIENT_ADDR]);

    let params = WithdrawBalanceParams {
        provider_or_client: CLIENT_ADDR,
        amount: TokenAmount::from_atto(1u8),
    };

    // caller is not the recipient
    rt.set_caller(*ACCOUNT_ACTOR_CODE_ID, Address::new_id(909));
    expect_abort(
        ExitCode::USR_FORBIDDEN,
        rt.call::<MarketActor>(
            Method::WithdrawBalance as u64,
            IpldBlock::serialize_cbor(&params).unwrap(),
        ),
    );
    rt.verify();

    // verify there was no withdrawal
    assert_eq!(TokenAmount::from_atto(20u8), get_balance(&rt, &CLIENT_ADDR).balance);

    check_state(&rt);
}

#[test]
fn balance_after_withdrawal_must_always_be_greater_than_or_equal_to_locked_amount() {
    let start_epoch = ChainEpoch::from(10);
    let end_epoch = start_epoch + 200 * EPOCHS_IN_DAY;
    let publish_epoch = ChainEpoch::from(5);

    let rt = setup();

    // publish the deal so that client AND provider collateral is locked
    rt.set_epoch(publish_epoch);
    let (_, deal) = generate_and_publish_deal(
        &rt,
        CLIENT_ADDR,
        &MinerAddresses::default(),
        start_epoch,
        end_epoch,
    );
    let provider_acct = get_balance(&rt, &PROVIDER_ADDR);
    assert_eq!(deal.provider_collateral, provider_acct.balance);
    assert_eq!(deal.provider_collateral, provider_acct.locked);
    let client_acct = get_balance(&rt, &CLIENT_ADDR);
    assert_eq!(deal.client_balance_requirement(), client_acct.balance);
    assert_eq!(deal.client_balance_requirement(), client_acct.locked);

    let withdraw_amount = TokenAmount::from_atto(1u8);
    let withdrawable_amount = TokenAmount::zero();
    // client cannot withdraw any funds since all it's balance is locked
    withdraw_client_balance(&rt, withdraw_amount.clone(), withdrawable_amount.clone(), CLIENT_ADDR);
    // provider cannot withdraw any funds since all it's balance is locked
    withdraw_provider_balance(
        &rt,
        withdraw_amount,
        withdrawable_amount,
        PROVIDER_ADDR,
        OWNER_ADDR,
        WORKER_ADDR,
    );

    // add some more funds to the provider & ensure withdrawal is limited by the locked funds
    let withdraw_amount = TokenAmount::from_atto(30u8);
    let withdrawable_amount = TokenAmount::from_atto(25u8);

    add_provider_funds(&rt, withdrawable_amount.clone(), &MinerAddresses::default());
    let provider_acct = get_balance(&rt, &PROVIDER_ADDR);
    assert_eq!(&deal.provider_collateral + &withdrawable_amount, provider_acct.balance);
    assert_eq!(deal.provider_collateral, provider_acct.locked);

    withdraw_provider_balance(
        &rt,
        withdraw_amount.clone(),
        withdrawable_amount.clone(),
        PROVIDER_ADDR,
        OWNER_ADDR,
        WORKER_ADDR,
    );

    // add some more funds to the client & ensure withdrawal is limited by the locked funds
    add_participant_funds(&rt, CLIENT_ADDR, withdrawable_amount.clone());
    let client_acct = get_balance(&rt, &CLIENT_ADDR);
    assert_eq!(deal.client_balance_requirement() + &withdrawable_amount, client_acct.balance);
    assert_eq!(deal.client_balance_requirement(), client_acct.locked);

    withdraw_client_balance(&rt, withdraw_amount, withdrawable_amount, CLIENT_ADDR);
    check_state(&rt);
}

#[test]
fn worker_balance_after_withdrawal_must_account_for_slashed_funds() {
    let start_epoch = ChainEpoch::from(10);
    let end_epoch = start_epoch + 200 * EPOCHS_IN_DAY;
    let publish_epoch = ChainEpoch::from(5);
    let sector_number = 7;
    let rt = setup();

    // publish deal
    rt.set_epoch(publish_epoch);
    let (deal_id, proposal) = generate_and_publish_deal(
        &rt,
        CLIENT_ADDR,
        &MinerAddresses::default(),
        start_epoch,
        end_epoch,
    );

    // activate the deal
    activate_deals(&rt, end_epoch + 1, PROVIDER_ADDR, publish_epoch, sector_number, &[deal_id]);
    let st = get_deal_state(&rt, deal_id);
    assert_eq!(publish_epoch, st.sector_start_epoch);

    // terminate the deal
    rt.set_epoch(publish_epoch + 1);
    terminate_deals(&rt, PROVIDER_ADDR, &[sector_number], &[deal_id]);
    assert_deal_deleted(&rt, deal_id, &proposal, sector_number);

    // provider cannot withdraw any funds since it's been terminated
    let withdraw_amount = TokenAmount::from_atto(1);
    let actual_withdrawn = TokenAmount::zero();
    withdraw_provider_balance(
        &rt,
        withdraw_amount,
        actual_withdrawn,
        PROVIDER_ADDR,
        OWNER_ADDR,
        WORKER_ADDR,
    );

    // add some more funds to the provider & ensure withdrawal is limited by the locked funds
    add_provider_funds(&rt, TokenAmount::from_atto(25), &MinerAddresses::default());
    let withdraw_amount = TokenAmount::from_atto(30);
    let actual_withdrawn = TokenAmount::from_atto(25);

    withdraw_provider_balance(
        &rt,
        withdraw_amount,
        actual_withdrawn,
        PROVIDER_ADDR,
        OWNER_ADDR,
        WORKER_ADDR,
    );

    check_state(&rt);
}

#[test]
fn adds_to_non_provider_funds() {
    struct TestCase {
        delta: u64,
        total: u64,
    }
    let test_cases = [
        TestCase { delta: 10, total: 10 },
        TestCase { delta: 20, total: 30 },
        TestCase { delta: 40, total: 70 },
    ];

    for caller_addr in &[CLIENT_ADDR, WORKER_ADDR] {
        let rt = setup();

        for tc in &test_cases {
            rt.set_caller(*ACCOUNT_ACTOR_CODE_ID, *caller_addr);
            rt.set_received(TokenAmount::from_atto(tc.delta));
            rt.expect_validate_caller_any();
            assert!(rt
                .call::<MarketActor>(
                    Method::AddBalance as u64,
                    IpldBlock::serialize_cbor(caller_addr).unwrap(),
                )
                .unwrap()
                .is_none());

            rt.verify();

            assert_eq!(get_balance(&rt, caller_addr).balance, TokenAmount::from_atto(tc.total));
            check_state(&rt);
        }
    }
}

#[test]
fn withdraws_from_provider_escrow_funds_and_sends_to_owner() {
    let rt = setup();

    let amount = TokenAmount::from_atto(20);
    add_provider_funds(&rt, amount.clone(), &MinerAddresses::default());

    assert_eq!(amount, get_balance(&rt, &PROVIDER_ADDR).balance);

    // worker calls WithdrawBalance, balance is transferred to owner
    let withdraw_amount = TokenAmount::from_atto(1);
    withdraw_provider_balance(
        &rt,
        withdraw_amount.clone(),
        withdraw_amount,
        PROVIDER_ADDR,
        OWNER_ADDR,
        WORKER_ADDR,
    );

    assert_eq!(TokenAmount::from_atto(19), get_balance(&rt, &PROVIDER_ADDR).balance);
    check_state(&rt);
}

#[test]
fn withdraws_from_non_provider_escrow_funds() {
    let rt = setup();

    let amount = TokenAmount::from_atto(20);
    add_participant_funds(&rt, CLIENT_ADDR, amount.clone());

    assert_eq!(get_balance(&rt, &CLIENT_ADDR).balance, amount);

    let withdraw_amount = TokenAmount::from_atto(1);
    withdraw_client_balance(&rt, withdraw_amount.clone(), withdraw_amount, CLIENT_ADDR);

    assert_eq!(get_balance(&rt, &CLIENT_ADDR).balance, TokenAmount::from_atto(19));
    check_state(&rt);
}

#[test]
fn client_withdrawing_more_than_escrow_balance_limits_to_available_funds() {
    let rt = setup();

    let amount = TokenAmount::from_atto(20);
    add_participant_funds(&rt, CLIENT_ADDR, amount.clone());

    // withdraw amount greater than escrow balance
    let withdraw_amount = TokenAmount::from_atto(25);
    withdraw_client_balance(&rt, withdraw_amount, amount, CLIENT_ADDR);

    assert_eq!(get_balance(&rt, &CLIENT_ADDR).balance, TokenAmount::zero());
    check_state(&rt);
}

#[test]
fn worker_withdrawing_more_than_escrow_balance_limits_to_available_funds() {
    let rt = setup();

    let amount = TokenAmount::from_atto(20);
    add_provider_funds(&rt, amount.clone(), &MinerAddresses::default());

    assert_eq!(get_balance(&rt, &PROVIDER_ADDR).balance, amount);

    let withdraw_amount = TokenAmount::from_atto(25);
    withdraw_provider_balance(&rt, withdraw_amount, amount, PROVIDER_ADDR, OWNER_ADDR, WORKER_ADDR);

    assert_eq!(get_balance(&rt, &PROVIDER_ADDR).balance, TokenAmount::zero());
    check_state(&rt);
}

#[test]
fn fail_when_balance_is_zero() {
    let rt = setup();

    rt.set_caller(*ACCOUNT_ACTOR_CODE_ID, OWNER_ADDR);
    rt.set_received(TokenAmount::zero());

    expect_abort(
        ExitCode::USR_ILLEGAL_ARGUMENT,
        rt.call::<MarketActor>(
            Method::AddBalance as u64,
            IpldBlock::serialize_cbor(&PROVIDER_ADDR).unwrap(),
        ),
    );

    rt.verify();
    check_state(&rt);
}

#[test]
fn fails_with_a_negative_withdraw_amount() {
    let rt = setup();

    let params = WithdrawBalanceParams {
        provider_or_client: PROVIDER_ADDR,
        amount: TokenAmount::from_atto(-1_i32),
    };

    expect_abort(
        ExitCode::USR_ILLEGAL_ARGUMENT,
        rt.call::<MarketActor>(
            Method::WithdrawBalance as u64,
            IpldBlock::serialize_cbor(&params).unwrap(),
        ),
    );

    rt.verify();
    check_state(&rt);
}

#[test]
fn fails_if_withdraw_from_provider_funds_is_not_initiated_by_the_owner_or_worker() {
    let rt = setup();

    let amount = TokenAmount::from_atto(20u8);
    add_provider_funds(&rt, amount.clone(), &MinerAddresses::default());

    assert_eq!(get_balance(&rt, &PROVIDER_ADDR).balance, amount);

    rt.expect_validate_caller_addr(vec![OWNER_ADDR, WORKER_ADDR]);
    let params = WithdrawBalanceParams {
        provider_or_client: PROVIDER_ADDR,
        amount: TokenAmount::from_atto(1u8),
    };

    // caller is not owner or worker
    rt.set_caller(*ACCOUNT_ACTOR_CODE_ID, Address::new_id(909));
    expect_provider_control_address(&rt, PROVIDER_ADDR, OWNER_ADDR, WORKER_ADDR);

    expect_abort(
        ExitCode::USR_FORBIDDEN,
        rt.call::<MarketActor>(
            Method::WithdrawBalance as u64,
            IpldBlock::serialize_cbor(&params).unwrap(),
        ),
    );
    rt.verify();

    // verify there was no withdrawal
    assert_eq!(get_balance(&rt, &PROVIDER_ADDR).balance, amount);
    check_state(&rt);
}

#[test]
fn deal_starts_on_day_boundary() {
    let mut policy = Policy::default();
    let interval = 288; // The mainnet value is too slow for testing.
    policy.deal_updates_interval = interval;
    let start_epoch = interval;
    let end_epoch = start_epoch + 200 * EPOCHS_IN_DAY;
    let publish_epoch = ChainEpoch::from(1);

    let mut rt = setup();
    rt.set_policy(policy);
    rt.set_epoch(publish_epoch);

    for i in 0..(3 * interval) {
        let piece_cid = make_piece_cid((format!("{i}")).as_bytes());
        let deal_id = generate_and_publish_deal_for_piece(
            &rt,
            CLIENT_ADDR,
            &MinerAddresses::default(),
            start_epoch,
            end_epoch,
            piece_cid,
            PaddedPieceSize(2048u64),
        );
        assert_eq!(i as DealID, deal_id);
    }

    // Check that DOBE has exactly 3 deals scheduled every epoch in the day following the start time
    let st: State = rt.get_state();
    let store = &rt.store;
    let dobe = st.load_deal_ops(store).unwrap();
    for e in interval..(2 * interval) {
        assert_n_good_deals(&dobe, interval, e, 3);
    }

    // DOBE has no deals scheduled in the previous or next day
    for e in 0..interval {
        assert_n_good_deals(&dobe, interval, e, 0);
    }
    for e in (2 * interval)..(3 * interval) {
        assert_n_good_deals(&dobe, interval, e, 0);
    }
}

#[test]
fn deal_starts_partway_through_day() {
    let mut policy = Policy::default();
    let interval = 288; // The mainnet value is too slow for testing.
    policy.deal_updates_interval = interval;
    let start_epoch = 100;
    let end_epoch = start_epoch + 200 * EPOCHS_IN_DAY;
    let publish_epoch = ChainEpoch::from(1);

    let mut rt = setup();
    rt.set_policy(policy);
    rt.set_epoch(publish_epoch);

    // First 100 deals (start_epoch % update interval) scheduled starting in the next period
    for i in 0..100 {
        let piece_cid = make_piece_cid((format!("{i}")).as_bytes());
        let deal_id = generate_and_publish_deal_for_piece(
            &rt,
            CLIENT_ADDR,
            &MinerAddresses::default(),
            start_epoch,
            end_epoch,
            piece_cid,
            PaddedPieceSize(2048u64),
        );
        assert_eq!(i as DealID, deal_id);
    }
    let st: State = rt.get_state();
    let store = &rt.store;
    let dobe = st.load_deal_ops(store).unwrap();
    for e in interval..(interval + start_epoch) {
        assert_n_good_deals(&dobe, interval, e, 1);
    }
    // Nothing scheduled between 0 and interval
    for e in 0..interval {
        assert_n_good_deals(&dobe, interval, e, 0);
    }

    // Now add another 50 deals
    for i in 100..150 {
        let piece_cid = make_piece_cid((format!("{i}")).as_bytes());
        let deal_id = generate_and_publish_deal_for_piece(
            &rt,
            CLIENT_ADDR,
            &MinerAddresses::default(),
            start_epoch,
            end_epoch,
            piece_cid,
            PaddedPieceSize(2048u64),
        );
        assert_eq!(i as DealID, deal_id);
    }
    let st: State = rt.get_state();
    let store = &rt.store;
    let dobe = st.load_deal_ops(store).unwrap();
    for e in start_epoch..(start_epoch + 50) {
        assert_n_good_deals(&dobe, interval, e, 1);
    }
}

#[test]
fn simple_deal() {
    let start_epoch = 1000;
    let end_epoch = start_epoch + 200 * EPOCHS_IN_DAY;
    let publish_epoch = ChainEpoch::from(1);

    let rt = setup();
    rt.set_epoch(publish_epoch);
    let next_allocation_id = 1;

    // Publish from miner worker.
    let mut deal1 = generate_deal_and_add_funds(
        &rt,
        CLIENT_ADDR,
        &MinerAddresses::default(),
        start_epoch,
        end_epoch,
    );
    deal1.verified_deal = false;
    rt.set_caller(*ACCOUNT_ACTOR_CODE_ID, WORKER_ADDR);
    let deal1_id = publish_deals(
        &rt,
        &MinerAddresses::default(),
        &[deal1],
        TokenAmount::zero(),
        next_allocation_id,
    )[0];

    // Publish from miner control address.
    let mut deal2 = generate_deal_and_add_funds(
        &rt,
        CLIENT_ADDR,
        &MinerAddresses::default(),
        start_epoch + 1,
        end_epoch + 1,
    );
    deal2.verified_deal = true;
    rt.set_caller(*ACCOUNT_ACTOR_CODE_ID, CONTROL_ADDR);
    let deal2_id = publish_deals(
        &rt,
        &MinerAddresses::default(),
        &[deal2.clone()],
        TokenAmount::from_whole(deal2.piece_size.0),
        next_allocation_id,
    )[0];

    // activate the deal
    activate_deals(&rt, end_epoch + 1, PROVIDER_ADDR, publish_epoch, 1, &[deal1_id, deal2_id]);
    let deal1st = get_deal_state(&rt, deal1_id);
    assert_eq!(publish_epoch, deal1st.sector_start_epoch);

    let deal2st = get_deal_state(&rt, deal2_id);
    assert_eq!(publish_epoch, deal2st.sector_start_epoch);

    check_state(&rt);
}

#[test]
fn deal_expires() {
    let start_epoch = 100;
    let end_epoch = start_epoch + 200 * EPOCHS_IN_DAY;
    let publish_epoch = ChainEpoch::from(1);

    let rt = setup();
    rt.set_epoch(publish_epoch);
    let next_allocation_id = 1;

    // Publish from miner worker.
    let mut deal = generate_deal_and_add_funds(
        &rt,
        CLIENT_ADDR,
        &MinerAddresses::default(),
        start_epoch,
        end_epoch,
    );
    deal.verified_deal = true;
    rt.set_caller(*ACCOUNT_ACTOR_CODE_ID, WORKER_ADDR);
    let deal_id = publish_deals(
        &rt,
        &MinerAddresses::default(),
        &[deal.clone()],
        TokenAmount::from_whole(deal.piece_size.0),
        next_allocation_id,
    )[0];

    rt.set_epoch(start_epoch + Policy::default().deal_updates_interval + 1);
    rt.expect_send_simple(
        BURNT_FUNDS_ACTOR_ADDR,
        METHOD_SEND,
        None,
        deal.provider_collateral,
        None,
        ExitCode::OK,
    );
    cron_tick(&rt);

    // No deal state for unactivated deal
    let st: State = rt.get_state();
    let states = DealMetaArray::load(&st.states, &rt.store).unwrap();
    assert!(states.get(deal_id).unwrap().is_none());

    // The proposal is gone
    assert!(DealArray::load(&st.proposals, &rt.store).unwrap().get(deal_id).unwrap().is_none());

    // Pending allocation ID is gone
    let pending_allocs = PendingDealAllocationsMap::load(
        &rt.store,
        &st.pending_deal_allocation_ids,
        PENDING_ALLOCATIONS_CONFIG,
        "pending allocations",
    )
    .unwrap();
    assert!(pending_allocs.get(&deal_id).unwrap().is_none());

    check_state(&rt);
}

#[test]
fn provider_and_client_addresses_are_resolved_before_persisting_state_and_sent_to_verigreg_actor_for_a_verified_deal(
) {
    use fvm_shared::address::BLS_PUB_LEN;

    // provider addresses
    let provider_bls = Address::new_bls(&[101; BLS_PUB_LEN]).unwrap();
    let provider_resolved = Address::new_id(112);

    // client addresses
    let client_bls = Address::new_bls(&[90; BLS_PUB_LEN]).unwrap();
    let client_resolved = Address::new_id(333);

    let rt = setup();
    rt.actor_code_cids.borrow_mut().insert(client_resolved, *ACCOUNT_ACTOR_CODE_ID);
    rt.actor_code_cids.borrow_mut().insert(provider_resolved, *MINER_ACTOR_CODE_ID);

    // mappings for resolving address
    rt.id_addresses.borrow_mut().insert(client_bls, client_resolved);
    rt.id_addresses.borrow_mut().insert(provider_bls, provider_resolved);

    // generate deal and add required funds for deal
    let start_epoch = 42;
    let end_epoch = start_epoch + 200 * EPOCHS_IN_DAY;
    rt.set_epoch(start_epoch);

    let mut deal = generate_deal_proposal(client_bls, provider_bls, start_epoch, end_epoch);
    deal.verified_deal = true;

    // add funds for client using its BLS address -> will be resolved and persisted
    let amount = deal.client_balance_requirement();

    rt.set_received(amount.clone());
    rt.set_caller(*ACCOUNT_ACTOR_CODE_ID, client_resolved);
    rt.expect_validate_caller_any();
    assert!(rt
        .call::<MarketActor>(
            Method::AddBalanceExported as u64,
            IpldBlock::serialize_cbor(&client_bls).unwrap(),
        )
        .is_ok());
    rt.verify();
    rt.add_balance(amount);

    assert_eq!(deal.client_balance_requirement(), get_balance(&rt, &client_resolved).balance);

    // add funds for provider using it's BLS address -> will be resolved and persisted
    rt.value_received.replace(deal.provider_collateral.clone());
    rt.set_caller(*ACCOUNT_ACTOR_CODE_ID, OWNER_ADDR);
    rt.expect_validate_caller_any();
    expect_provider_control_address(&rt, provider_resolved, OWNER_ADDR, WORKER_ADDR);

    assert!(rt
        .call::<MarketActor>(
            Method::AddBalance as u64,
            IpldBlock::serialize_cbor(&provider_bls).unwrap(),
        )
        .unwrap()
        .is_none());
    rt.verify();
    rt.add_balance(deal.provider_collateral.clone());
    assert_eq!(deal.provider_collateral, get_balance(&rt, &provider_resolved).balance);

    // publish deal using the BLS addresses
    rt.set_caller(*ACCOUNT_ACTOR_CODE_ID, WORKER_ADDR);
    rt.expect_validate_caller_any();

    expect_provider_is_control_address(&rt, provider_resolved, WORKER_ADDR, true);
    expect_query_network_info(&rt);

    //  create a client proposal with a valid signature
    let st: State = rt.get_state();
    let deal_id = st.next_id;
    let mut params = PublishStorageDealsParams { deals: vec![] };
    let buf = RawBytes::serialize(&deal).expect("failed to marshal deal proposal");
    let sig = Signature::new_bls(buf.to_vec());
    let client_proposal = ClientDealProposal { client_signature: sig, proposal: deal.clone() };
    params.deals.push(client_proposal);
    // expect a call to verify the above signature

    let auth_param = IpldBlock::serialize_cbor(&AuthenticateMessageParams {
        signature: buf.to_vec(),
        message: buf.to_vec(),
    })
    .unwrap();

    rt.expect_send(
        deal.client,
        AUTHENTICATE_MESSAGE_METHOD,
        auth_param,
        TokenAmount::zero(),
        None,
        SendFlags::READ_ONLY,
        AUTHENTICATE_MESSAGE_RESPONSE.clone(),
        ExitCode::OK,
        None,
    );

    // Data cap transfer is requested using the resolved address (not that it matters).
    let alloc_req = ext::verifreg::AllocationRequests {
        allocations: vec![AllocationRequest {
            provider: provider_resolved.id().unwrap(),
            data: deal.piece_cid,
            size: deal.piece_size,
            term_min: deal.end_epoch - deal.start_epoch,
            term_max: (deal.end_epoch - deal.start_epoch) + 90 * EPOCHS_IN_DAY,
            expiration: deal.start_epoch,
        }],
        extensions: vec![],
    };
    let balance_of_params = client_resolved;
    let balance_of_return = TokenAmount::from_whole(2048);
    rt.expect_send_simple(
        DATACAP_TOKEN_ACTOR_ADDR,
        ext::datacap::BALANCE_OF_METHOD,
        IpldBlock::serialize_cbor(&balance_of_params).unwrap(),
        TokenAmount::zero(),
        IpldBlock::serialize_cbor(&balance_of_return).unwrap(),
        ExitCode::OK,
    );

    let datacap_amount = TokenAmount::from_whole(deal.piece_size.0 as i64);
    let transfer_params = TransferFromParams {
        from: client_resolved,
        to: VERIFIED_REGISTRY_ACTOR_ADDR,
        amount: datacap_amount.clone(),
        operator_data: serialize(&alloc_req, "allocation requests").unwrap(),
    };
    let transfer_return = TransferFromReturn {
        from_balance: TokenAmount::zero(),
        to_balance: datacap_amount,
        allowance: TokenAmount::zero(),
        recipient_data: serialize(
            &AllocationsResponse {
                allocation_results: BatchReturn::ok(1),
                extension_results: BatchReturn::empty(),
                new_allocations: vec![1],
            },
            "allocations response",
        )
        .unwrap(),
    };
    rt.expect_send_simple(
        DATACAP_TOKEN_ACTOR_ADDR,
        ext::datacap::TRANSFER_FROM_METHOD,
        IpldBlock::serialize_cbor(&transfer_params).unwrap(),
        TokenAmount::zero(),
        IpldBlock::serialize_cbor(&transfer_return).unwrap(),
        ExitCode::OK,
    );
    let mut normalized_deal = deal;
    normalized_deal.provider = provider_resolved;
    normalized_deal.client = client_resolved;
    let normalized_proposal_bytes =
        RawBytes::serialize(&normalized_deal).expect("failed to marshal deal proposal");
    let notify_param = IpldBlock::serialize_cbor(&MarketNotifyDealParams {
        proposal: normalized_proposal_bytes.to_vec(),
        deal_id,
    })
    .unwrap();
    rt.expect_send_simple(
        client_resolved,
        MARKET_NOTIFY_DEAL_METHOD,
        notify_param,
        TokenAmount::zero(),
        None,
        ExitCode::OK,
    );

    expect_emitted(
        &rt,
        "deal-published",
        deal_id,
        client_resolved.id().unwrap(),
        provider_resolved.id().unwrap(),
    );

    let ret: PublishStorageDealsReturn = rt
        .call::<MarketActor>(
            Method::PublishStorageDeals as u64,
            IpldBlock::serialize_cbor(&params).unwrap(),
        )
        .unwrap()
        .unwrap()
        .deserialize()
        .unwrap();
    rt.verify();
    let deal_id = ret.ids[0];

    // assert that deal is persisted with the resolved addresses
    let prop = get_deal_proposal(&rt, deal_id);
    assert_eq!(client_resolved, prop.client);
    assert_eq!(provider_resolved, prop.provider);

    check_state(&rt);
}

#[test]
fn datacap_transfers_batched() {
    let rt = setup();
    let start_epoch = 42;
    let end_epoch = start_epoch + 200 * EPOCHS_IN_DAY;
    rt.set_epoch(start_epoch);

    let client1_addr = Address::new_id(900);
    let client2_addr = Address::new_id(901);

    // Propose two deals for client1, and one for client2.
    let mut deal1 = generate_deal_and_add_funds(
        &rt,
        client1_addr,
        &MinerAddresses::default(),
        start_epoch,
        end_epoch,
    );
    let mut deal2 = generate_deal_and_add_funds(
        &rt,
        client1_addr,
        &MinerAddresses::default(),
        start_epoch,
        end_epoch + 1,
    );
    let mut deal3 = generate_deal_and_add_funds(
        &rt,
        client2_addr,
        &MinerAddresses::default(),
        start_epoch,
        end_epoch,
    );
    deal1.verified_deal = true;
    deal2.verified_deal = true;
    deal3.verified_deal = true;
    let datacap_balance = TokenAmount::from_whole(deal1.piece_size.0 * 10);

    rt.set_caller(*ACCOUNT_ACTOR_CODE_ID, WORKER_ADDR);
    let ids =
        publish_deals(&rt, &MinerAddresses::default(), &[deal1, deal2, deal3], datacap_balance, 1);
    assert_eq!(3, ids.len());

    check_state(&rt);
}

#[test]
fn datacap_transfer_drops_deal_when_cap_insufficient() {
    let rt = setup();
    let start_epoch = 42;
    let end_epoch = start_epoch + 200 * EPOCHS_IN_DAY;
    let client1_addr = Address::new_id(900);
    rt.set_epoch(start_epoch);

    let mut deal1 = generate_deal_and_add_funds(
        &rt,
        client1_addr,
        &MinerAddresses::default(),
        start_epoch,
        end_epoch,
    );
    let mut deal2 = generate_deal_and_add_funds(
        &rt,
        client1_addr,
        &MinerAddresses::default(),
        start_epoch,
        end_epoch + 1,
    );
    deal1.verified_deal = true;
    deal2.verified_deal = true;
    let datacap_balance = TokenAmount::from_whole(deal1.piece_size.0); // Enough for 1 deal

    rt.set_caller(*ACCOUNT_ACTOR_CODE_ID, WORKER_ADDR);
    let ids = publish_deals(
        &rt,
        &MinerAddresses::default(),
        &[deal1, deal2],
        datacap_balance,
        1, // Only 1
    );
    assert_eq!(1, ids.len());

    check_state(&rt);
}

#[test]
fn publish_a_deal_after_activating_a_previous_deal_which_has_a_start_epoch_far_in_the_future() {
    let start_epoch = 1000;
    let end_epoch = start_epoch + 200 * EPOCHS_IN_DAY;
    let publish_epoch = ChainEpoch::from(1);

    let rt = setup();

    // publish the deal and activate it
    rt.set_epoch(publish_epoch);
    let (deal1, _) = generate_and_publish_deal(
        &rt,
        CLIENT_ADDR,
        &MinerAddresses::default(),
        start_epoch,
        end_epoch,
    );
    activate_deals(&rt, end_epoch, PROVIDER_ADDR, publish_epoch, 1, &[deal1]);
    let st = get_deal_state(&rt, deal1);
    assert_eq!(publish_epoch, st.sector_start_epoch);

    // now publish a second deal and activate it
    let new_epoch = publish_epoch + 1;
    rt.set_epoch(new_epoch);
    let (deal2, _) = generate_and_publish_deal(
        &rt,
        CLIENT_ADDR,
        &MinerAddresses::default(),
        start_epoch + 1,
        end_epoch + 1,
    );
    activate_deals(&rt, end_epoch + 1, PROVIDER_ADDR, new_epoch, 2, &[deal2]);
    check_state(&rt);
}

#[test]
fn publish_a_deal_with_enough_collateral_when_circulating_supply_is_superior_to_zero() {
    let policy = Policy::default();

    let start_epoch = 1000;
    let end_epoch = start_epoch + 200 * EPOCHS_IN_DAY;
    let publish_epoch = ChainEpoch::from(1);

    let rt = setup();

    let client_collateral = TokenAmount::from_atto(10u8); // min is zero so this is placeholder

    // given power and circ supply cancel this should be 1*dealqapower / 100
    let deal_size = PaddedPieceSize(2048u64); // generateDealProposal's deal size
    let provider_collateral = TokenAmount::from_atto(
        (deal_size.0 * (policy.prov_collateral_percent_supply_num as u64))
            / policy.prov_collateral_percent_supply_denom as u64,
    );

    let deal = generate_deal_with_collateral_and_add_funds(
        &rt,
        CLIENT_ADDR,
        &MinerAddresses::default(),
        provider_collateral,
        client_collateral,
        start_epoch,
        end_epoch,
    );
    let qa_power = StoragePower::from_i128(1 << 50).unwrap();
    rt.set_circulating_supply(TokenAmount::from_atto(qa_power)); // convenient for these two numbers to cancel out

    // publish the deal successfully
    rt.set_epoch(publish_epoch);
    rt.set_caller(*ACCOUNT_ACTOR_CODE_ID, WORKER_ADDR);
    let ids = publish_deals(&rt, &MinerAddresses::default(), &[deal], TokenAmount::zero(), 1);
    assert_eq!(1, ids.len());
    check_state(&rt);
}

#[test]
fn publish_multiple_deals_for_different_clients_and_ensure_balances_are_correct() {
    let start_epoch = 42;
    let end_epoch = start_epoch + 200 * EPOCHS_IN_DAY;

    let rt = setup();

    let client1_addr = Address::new_id(900);
    let client2_addr = Address::new_id(901);
    let client3_addr = Address::new_id(902);

    // generate first deal for
    let deal1 = generate_deal_and_add_funds(
        &rt,
        client1_addr,
        &MinerAddresses::default(),
        start_epoch,
        end_epoch,
    );

    // generate second deal
    let deal2 = generate_deal_and_add_funds(
        &rt,
        client2_addr,
        &MinerAddresses::default(),
        start_epoch,
        end_epoch,
    );

    // generate third deal
    let deal3 = generate_deal_and_add_funds(
        &rt,
        client3_addr,
        &MinerAddresses::default(),
        start_epoch,
        end_epoch,
    );

    rt.set_caller(*ACCOUNT_ACTOR_CODE_ID, WORKER_ADDR);
    let ids = publish_deals(
        &rt,
        &MinerAddresses::default(),
        &[deal1.clone(), deal2.clone(), deal3.clone()],
        TokenAmount::zero(),
        1,
    );
    assert_eq!(3, ids.len());

    // assert locked balance for all clients and provider
    let provider_locked_expected =
        &deal1.provider_collateral + &deal2.provider_collateral + &deal3.provider_collateral;
    let client1_locked = get_balance(&rt, &client1_addr).locked;
    let client2_locked = get_balance(&rt, &client2_addr).locked;
    let client3_locked = get_balance(&rt, &client3_addr).locked;
    assert_eq!(deal1.client_balance_requirement(), client1_locked);
    assert_eq!(deal2.client_balance_requirement(), client2_locked);
    assert_eq!(deal3.client_balance_requirement(), client3_locked);
    assert_eq!(provider_locked_expected, get_balance(&rt, &PROVIDER_ADDR).locked);

    // assert locked funds dealStates
    let st: State = rt.get_state();
    let total_client_collateral_locked =
        &deal3.client_collateral + &deal2.client_collateral + &deal2.client_collateral;
    assert_eq!(total_client_collateral_locked, st.total_client_locked_collateral);
    assert_eq!(provider_locked_expected, st.total_provider_locked_collateral);
    let total_storage_fee =
        &deal1.total_storage_fee() + &deal2.total_storage_fee() + &deal3.total_storage_fee();
    assert_eq!(total_storage_fee, st.total_client_storage_fee);

    // publish two more deals for same clients with same provider
    let deal4 = generate_deal_and_add_funds(
        &rt,
        client3_addr,
        &MinerAddresses::default(),
        1000,
        1000 + 200 * EPOCHS_IN_DAY,
    );
    let deal5 = generate_deal_and_add_funds(
        &rt,
        client3_addr,
        &MinerAddresses::default(),
        100,
        100 + 200 * EPOCHS_IN_DAY,
    );
    rt.set_caller(*ACCOUNT_ACTOR_CODE_ID, WORKER_ADDR);
    let ids = publish_deals(
        &rt,
        &MinerAddresses::default(),
        &[deal4.clone(), deal5.clone()],
        TokenAmount::zero(),
        1,
    );
    assert_eq!(2, ids.len());

    // assert locked balances for clients and provider
    let provider_locked_expected =
        &provider_locked_expected + &deal4.provider_collateral + &deal5.provider_collateral;
    assert_eq!(provider_locked_expected, get_balance(&rt, &PROVIDER_ADDR).locked);

    let client3_locked_updated = get_balance(&rt, &client3_addr).locked;
    assert_eq!(
        &client3_locked + &deal4.client_balance_requirement() + &deal5.client_balance_requirement(),
        client3_locked_updated
    );

    let client1_locked = get_balance(&rt, &client1_addr).locked;
    let client2_locked = get_balance(&rt, &client2_addr).locked;
    assert_eq!(deal1.client_balance_requirement(), client1_locked);
    assert_eq!(deal2.client_balance_requirement(), client2_locked);

    // assert locked funds dealStates
    let st: State = rt.get_state();
    let total_client_collateral_locked =
        &total_client_collateral_locked + &deal4.client_collateral + &deal5.client_collateral;
    assert_eq!(total_client_collateral_locked, st.total_client_locked_collateral);
    assert_eq!(provider_locked_expected, st.total_client_locked_collateral);

    let total_storage_fee =
        &total_storage_fee + &deal4.total_storage_fee() + &deal5.total_storage_fee();
    assert_eq!(total_storage_fee, st.total_client_storage_fee);

    // PUBLISH DEALS with a different provider
    let provider2_addr = Address::new_id(109);

    // generate first deal for second provider
    let addrs = MinerAddresses { provider: provider2_addr, ..MinerAddresses::default() };
    let deal6 =
        generate_deal_and_add_funds(&rt, client1_addr, &addrs, 20, 20 + 200 * EPOCHS_IN_DAY);

    // generate second deal for second provider
    let deal7 =
        generate_deal_and_add_funds(&rt, client1_addr, &addrs, 25, 60 + 200 * EPOCHS_IN_DAY);

    // publish both the deals for the second provider
    rt.set_caller(*ACCOUNT_ACTOR_CODE_ID, WORKER_ADDR);
    let ids = publish_deals(&rt, &addrs, &[deal6.clone(), deal7.clone()], TokenAmount::zero(), 1);
    assert_eq!(2, ids.len());

    // assertions
    let st: State = rt.get_state();
    let provider2_locked = &deal6.provider_collateral + &deal7.provider_collateral;
    assert_eq!(provider2_locked, get_balance(&rt, &provider2_addr).locked);
    let client1_locked_updated = get_balance(&rt, &client1_addr).locked;
    assert_eq!(
        &deal7.client_balance_requirement() + &client1_locked + &deal6.client_balance_requirement(),
        client1_locked_updated
    );

    // assert first provider's balance as well
    assert_eq!(provider_locked_expected, get_balance(&rt, &PROVIDER_ADDR).locked);

    let total_client_collateral_locked =
        &total_client_collateral_locked + &deal6.client_collateral + &deal7.client_collateral;
    assert_eq!(total_client_collateral_locked, st.total_client_locked_collateral);
    assert_eq!(provider_locked_expected + provider2_locked, st.total_provider_locked_collateral);
    let total_storage_fee =
        &total_storage_fee + &deal6.total_storage_fee() + &deal7.total_storage_fee();
    assert_eq!(total_storage_fee, st.total_client_storage_fee);
    check_state(&rt);
}

#[test]
fn active_deals_multiple_times_with_different_providers() {
    let start_epoch = 10;
    let end_epoch = start_epoch + 200 * EPOCHS_IN_DAY;
    let current_epoch = ChainEpoch::from(5);
    let sector_expiry = end_epoch + 100;

    let rt = setup();
    rt.set_epoch(current_epoch);

    // provider 1 publishes deals1 and deals2 and deal3
    let deal1 = generate_and_publish_deal(
        &rt,
        CLIENT_ADDR,
        &MinerAddresses::default(),
        start_epoch,
        end_epoch,
    )
    .0;
    let deal2 = generate_and_publish_deal(
        &rt,
        CLIENT_ADDR,
        &MinerAddresses::default(),
        start_epoch,
        end_epoch + 1,
    )
    .0;
    let deal3 = generate_and_publish_deal(
        &rt,
        CLIENT_ADDR,
        &MinerAddresses::default(),
        start_epoch,
        end_epoch + 2,
    )
    .0;

    // provider2 publishes deal4 and deal5
    let provider2_addr = Address::new_id(401);
    let addrs = MinerAddresses { provider: provider2_addr, ..MinerAddresses::default() };
    let deal4 = generate_and_publish_deal(&rt, CLIENT_ADDR, &addrs, start_epoch, end_epoch).0;
    let deal5 = generate_and_publish_deal(&rt, CLIENT_ADDR, &addrs, start_epoch, end_epoch + 1).0;

    // provider1 activates deal1 and deal2 but that does not activate deal3 to deal5
    activate_deals(&rt, sector_expiry, PROVIDER_ADDR, current_epoch, 1, &[deal1, deal2]);
    assert_deals_not_activated(&rt, current_epoch, &[deal3, deal4, deal5]);

    // provider2 activates deal5 but that does not activate deal3 or deal4
    activate_deals(&rt, sector_expiry, provider2_addr, current_epoch, 1, &[deal5]);
    assert_deals_not_activated(&rt, current_epoch, &[deal3, deal4]);

    // provider1 activates deal3
    activate_deals(&rt, sector_expiry, PROVIDER_ADDR, current_epoch, 2, &[deal3]);
    assert_deals_not_activated(&rt, current_epoch, &[deal4]);
    check_state(&rt);
}

#[test]
fn terminating_a_deal_removes_proposal_synchronously() {
    let start_epoch = 50;
    let end_epoch = start_epoch + 200 * EPOCHS_IN_DAY;
    let sector_expiry = end_epoch + 100;
    let sector_number = 7;
    let rt = setup();
    let addrs = &MinerAddresses::default();

    let (deal_id, proposal) = publish_and_activate_deal(
        &rt,
        CLIENT_ADDR,
        addrs,
        sector_number,
        start_epoch,
        end_epoch,
        0,
        sector_expiry,
    );

    // terminating the deal deletes proposal, state and pending_proposal but leaves deal op in queue
    terminate_deals(&rt, addrs.provider, &[sector_number], &[deal_id]);
    assert_deal_deleted(&rt, deal_id, &proposal, sector_number);
    check_state(&rt);

    // the next cron_tick will remove the dangling deal op entry
    rt.set_epoch(process_epoch(start_epoch, deal_id));
    cron_tick(&rt);
    check_state(&rt);
}

#[test]
fn settling_deal_fails_when_deal_update_epoch_is_in_the_future() {
    let start_epoch = 50;
    let end_epoch = start_epoch + 200 * EPOCHS_IN_DAY;
    let sector_expiry = end_epoch + 100;
    let sector_number = 7;
    let rt = setup();

    let (deal_id, _) = publish_and_activate_deal(
        &rt,
        CLIENT_ADDR,
        &MinerAddresses::default(),
        sector_number,
        start_epoch,
        end_epoch,
        0,
        sector_expiry,
    );

    // update last updated to some time in the future (breaks state invariants)
    update_last_updated(&rt, deal_id, end_epoch + 1000);

    // set current epoch of the deal to the end epoch so it's picked up for "processing" in the next cron tick.
    rt.set_epoch(end_epoch);
    let ret = settle_deal_payments(&rt, MinerAddresses::default().provider, &[deal_id], &[], &[]);
    assert_eq!(ret.results.codes(), &[ExitCode::USR_ILLEGAL_STATE]);

    check_state_with_expected(
        &rt,
        &[Regex::new("deal \\d+ last updated epoch \\d+ after current \\d+").unwrap()],
    );
}

#[test]
fn settling_payments_for_a_deal_at_its_start_epoch_results_in_zero_payment_and_no_slashing() {
    let start_epoch = ChainEpoch::from(50);
    let end_epoch = start_epoch + 200 * EPOCHS_IN_DAY;
    let sector_expiry = end_epoch + 100;
    let sector_number = 7;

    // set start epoch to coincide with processing (0 + 0 % 2880 = 0)
    let start_epoch = 0;
    let rt = setup();
    let (deal_id, _) = publish_and_activate_deal(
        &rt,
        CLIENT_ADDR,
        &MinerAddresses::default(),
        sector_number,
        start_epoch,
        end_epoch,
        0,
        sector_expiry,
    );

    // move the current epoch to start
    rt.set_epoch(start_epoch);
    let (pay, slashed) = settle_deal_payments_and_assert_balances(
        &rt,
        CLIENT_ADDR,
        MinerAddresses::default().provider,
        start_epoch,
        deal_id,
        false,
    );
    assert_eq!(TokenAmount::zero(), pay);
    assert_eq!(TokenAmount::zero(), slashed);

    // deal proposal and state should NOT be deleted
    get_deal_proposal(&rt, deal_id);
    get_deal_state(&rt, deal_id);
    check_state(&rt);
}

#[test]
fn cannot_publish_the_same_deal_twice_before_a_cron_tick() {
    let start_epoch = ChainEpoch::from(50);
    let end_epoch = start_epoch + 200 * EPOCHS_IN_DAY;

    // Publish a deal
    let rt = setup();
    generate_and_publish_deal(&rt, CLIENT_ADDR, &MinerAddresses::default(), start_epoch, end_epoch);

    // now try to publish it again and it should fail because it will still be in pending state
    let d2 = generate_deal_and_add_funds(
        &rt,
        CLIENT_ADDR,
        &MinerAddresses::default(),
        start_epoch,
        end_epoch,
    );
    let buf = RawBytes::serialize(d2.clone()).expect("failed to marshal deal proposal");
    let sig = Signature::new_bls(buf.to_vec());
    let params = PublishStorageDealsParams {
        deals: vec![ClientDealProposal { proposal: d2.clone(), client_signature: sig }],
    };
    rt.expect_validate_caller_any();
    expect_provider_is_control_address(&rt, PROVIDER_ADDR, WORKER_ADDR, true);
    expect_query_network_info(&rt);
    rt.set_caller(*ACCOUNT_ACTOR_CODE_ID, WORKER_ADDR);

    let auth_param = IpldBlock::serialize_cbor(&AuthenticateMessageParams {
        signature: buf.to_vec(),
        message: buf.to_vec(),
    })
    .unwrap();

    rt.expect_send(
        d2.client,
        AUTHENTICATE_MESSAGE_METHOD,
        auth_param,
        TokenAmount::zero(),
        None,
        SendFlags::READ_ONLY,
        AUTHENTICATE_MESSAGE_RESPONSE.clone(),
        ExitCode::OK,
        None,
    );

    expect_abort(
        ExitCode::USR_ILLEGAL_ARGUMENT,
        rt.call::<MarketActor>(
            Method::PublishStorageDeals as u64,
            IpldBlock::serialize_cbor(&params).unwrap(),
        ),
    );
    rt.verify();
    check_state(&rt);
}

#[test]
fn fail_when_current_epoch_greater_than_start_epoch_of_deal() {
    let start_epoch = 10;
    let end_epoch = start_epoch + 200 * EPOCHS_IN_DAY;
    let sector_expiry = end_epoch + 100;
    let sector_number = 7;

    let rt = setup();
    let (deal_id, _) = generate_and_publish_deal(
        &rt,
        CLIENT_ADDR,
        &MinerAddresses::default(),
        start_epoch,
        end_epoch,
    );

    rt.set_epoch(start_epoch + 1);
    let res = batch_activate_deals_raw(
        &rt,
        PROVIDER_ADDR,
        vec![SectorDeals {
            sector_number,
            sector_expiry,
            sector_type: RegisteredSealProof::StackedDRG8MiBV1,
            deal_ids: vec![deal_id],
        }],
        false,
        &[],
    )
    .unwrap();

    let res: BatchActivateDealsResult =
        res.unwrap().deserialize().expect("VerifyDealsForActivation failed!");

    assert_eq!(res.activation_results.codes(), vec![EX_DEAL_EXPIRED]);

    rt.verify();
    check_state(&rt);
}

#[test]
fn fail_when_end_epoch_of_deal_greater_than_sector_expiry() {
    let start_epoch = 10;
    let end_epoch = start_epoch + 200 * EPOCHS_IN_DAY;
    let sector_number = 7;

    let rt = setup();
    let (deal_id, _) = generate_and_publish_deal(
        &rt,
        CLIENT_ADDR,
        &MinerAddresses::default(),
        start_epoch,
        end_epoch,
    );

    let res = batch_activate_deals_raw(
        &rt,
        PROVIDER_ADDR,
        vec![SectorDeals {
            sector_number,
            sector_expiry: end_epoch - 1,
            sector_type: RegisteredSealProof::StackedDRG8MiBV1,
            deal_ids: vec![deal_id],
        }],
        false,
        &[],
    )
    .unwrap();

    let res: BatchActivateDealsResult =
        res.unwrap().deserialize().expect("VerifyDealsForActivation failed!");

    assert_eq!(res.activation_results.codes(), vec![ExitCode::USR_ILLEGAL_ARGUMENT]);

    rt.verify();
    check_state(&rt);
}

#[test]
fn fail_to_activate_all_deals_if_one_deal_fails() {
    let start_epoch = 10;
    let end_epoch = start_epoch + 200 * EPOCHS_IN_DAY;
    let sector_expiry = end_epoch + 100;
    let sector_number = 7;

    let rt = setup();
    // activate deal1 so it fails later
    let (deal_id1, _) = generate_and_publish_deal(
        &rt,
        CLIENT_ADDR,
        &MinerAddresses::default(),
        start_epoch,
        end_epoch,
    );
    batch_activate_deals(
        &rt,
        PROVIDER_ADDR,
        &[(sector_number, sector_expiry, vec![deal_id1])],
        false,
    );

    let (deal_id2, _) = generate_and_publish_deal(
        &rt,
        CLIENT_ADDR,
        &MinerAddresses::default(),
        start_epoch,
        end_epoch + 1,
    );

    let res = batch_activate_deals_raw(
        &rt,
        PROVIDER_ADDR,
        vec![SectorDeals {
            sector_number,
            sector_expiry,
            sector_type: RegisteredSealProof::StackedDRG8MiBV1,
            deal_ids: vec![deal_id1, deal_id2],
        }],
        false,
        &[],
    )
    .unwrap();
    let res: BatchActivateDealsResult =
        res.unwrap().deserialize().expect("VerifyDealsForActivation failed!");

    assert_eq!(res.activation_results.codes(), vec![ExitCode::USR_ILLEGAL_ARGUMENT]);
    rt.verify();

    // no state for deal2 means deal2 activation has failed
    let st: State = rt.get_state();

    let states = DealMetaArray::load(&st.states, &rt.store).unwrap();

    let s = states.get(deal_id2).unwrap();
    assert!(s.is_none());
    check_state(&rt);
}

#[test]
fn locked_fund_tracking_states() {
    let p1 = Address::new_id(201);
    let p2 = Address::new_id(202);
    let p3 = Address::new_id(203);

    let c1 = Address::new_id(104);
    let c2 = Address::new_id(105);
    let c3 = Address::new_id(106);

    let m1 = MinerAddresses {
        owner: OWNER_ADDR,
        worker: WORKER_ADDR,
        provider: p1,
        control: vec![CONTROL_ADDR],
    };
    let m2 = MinerAddresses {
        owner: OWNER_ADDR,
        worker: WORKER_ADDR,
        provider: p2,
        control: vec![CONTROL_ADDR],
    };
    let m3 = MinerAddresses {
        owner: OWNER_ADDR,
        worker: WORKER_ADDR,
        provider: p3,
        control: vec![CONTROL_ADDR],
    };

    let start_epoch = ChainEpoch::from(2880);
    let end_epoch = start_epoch + 200 * EPOCHS_IN_DAY;
    let sector_expiry = end_epoch + 400;

    let rt = setup();
    rt.actor_code_cids.borrow_mut().insert(p1, *MINER_ACTOR_CODE_ID);
    rt.actor_code_cids.borrow_mut().insert(c1, *ACCOUNT_ACTOR_CODE_ID);
    let st: State = rt.get_state();

    // assert values are zero
    assert!(st.total_client_locked_collateral.is_zero());
    assert!(st.total_provider_locked_collateral.is_zero());
    assert!(st.total_client_storage_fee.is_zero());

    // Publish deal1, deal2, and deal3 with different client and provider
    let (deal_id1, d1) = generate_and_publish_deal(&rt, c1, &m1, start_epoch, end_epoch);

    let (deal_id2, d2) = generate_and_publish_deal(&rt, c2, &m2, start_epoch, end_epoch);

    let (deal_id3, d3) = generate_and_publish_deal(&rt, c3, &m3, start_epoch, end_epoch);

    let csf = d1.total_storage_fee() + d2.total_storage_fee() + d3.total_storage_fee();
    let plc = &d1.provider_collateral + d2.provider_collateral + &d3.provider_collateral;
    let clc = d1.client_collateral + d2.client_collateral + &d3.client_collateral;

    assert_locked_fund_states(&rt, csf.clone(), plc.clone(), clc.clone());

    // activation doesn't change anything
    let curr = rt.set_epoch(start_epoch - 1);
    // Providers happened to use the same sector number.
    let sector_number = 7;
    activate_deals(&rt, sector_expiry, p1, curr, sector_number, &[deal_id1]);
    activate_deals(&rt, sector_expiry, p2, curr, sector_number, &[deal_id2]);

    assert_locked_fund_states(&rt, csf.clone(), plc.clone(), clc.clone());

    // make payment for p1 and p2, p3 times out as it has not been activated
    let curr = rt.set_epoch(curr + 100);
    let last_payment_epoch = curr;
    rt.expect_send_simple(
        BURNT_FUNDS_ACTOR_ADDR,
        METHOD_SEND,
        None,
        d3.provider_collateral.clone(),
        None,
        ExitCode::OK,
    );
    settle_deal_payments(&rt, OWNER_ADDR, &[deal_id1, deal_id2, deal_id3], &[], &[]);
    let duration = curr - start_epoch;
    let payment: TokenAmount = 2 * &d1.storage_price_per_epoch * duration;
    let mut csf = (csf - payment) - d3.total_storage_fee();
    let mut plc = plc - d3.provider_collateral;
    let mut clc = clc - d3.client_collateral;
    assert_locked_fund_states(&rt, csf.clone(), plc.clone(), clc.clone());

    // one more round of payment for deal1 and deal2
    let curr = rt.set_epoch(curr + 100);
    let duration = curr - last_payment_epoch;
    let payment = 2 * d1.storage_price_per_epoch * duration;
    csf -= payment;
    settle_deal_payments(&rt, OWNER_ADDR, &[deal_id1, deal_id2, deal_id3], &[], &[]);
    assert_locked_fund_states(&rt, csf.clone(), plc.clone(), clc.clone());

    // terminate deal1
    rt.set_epoch(curr + 1);
    terminate_deals(&rt, m1.provider, &[sector_number], &[deal_id1]);

    // attempt to settle payments which terminates deal1 and expires deal2
    rt.set_epoch(end_epoch);
    csf = TokenAmount::zero();
    clc = TokenAmount::zero();
    plc = TokenAmount::zero();
    settle_deal_payments(&rt, OWNER_ADDR, &[deal_id1, deal_id2, deal_id3], &[deal_id2], &[]);
    assert_locked_fund_states(&rt, csf, plc, clc);
    check_state(&rt);
}

fn assert_locked_fund_states(
    rt: &MockRuntime,
    storage_fee: TokenAmount,
    provider_collateral: TokenAmount,
    client_collateral: TokenAmount,
) {
    let st: State = rt.get_state();

    assert_eq!(client_collateral, st.total_client_locked_collateral);
    assert_eq!(provider_collateral, st.total_provider_locked_collateral);
    assert_eq!(storage_fee, st.total_client_storage_fee);
}

#[allow(dead_code)]
fn market_actor_deals() {
    let rt = setup();
    let miner_addresses = MinerAddresses {
        owner: OWNER_ADDR,
        worker: WORKER_ADDR,
        provider: PROVIDER_ADDR,
        control: vec![],
    };

    // test adding provider funds
    let funds = TokenAmount::from_atto(20_000_000);
    add_provider_funds(&rt, funds.clone(), &MinerAddresses::default());
    assert_eq!(funds, get_balance(&rt, &PROVIDER_ADDR).balance);

    add_participant_funds(&rt, CLIENT_ADDR, funds);
    let mut deal_proposal =
        generate_deal_proposal(CLIENT_ADDR, PROVIDER_ADDR, 1, 200 * EPOCHS_IN_DAY);

    // First attempt at publishing the deal should work
    rt.set_caller(*ACCOUNT_ACTOR_CODE_ID, WORKER_ADDR);
    let ids =
        publish_deals(&rt, &miner_addresses, &[deal_proposal.clone()], TokenAmount::zero(), 1);
    assert_eq!(1, ids.len());

    // Second attempt at publishing the same deal should fail
    publish_deals_expect_abort(
        &rt,
        &miner_addresses,
        deal_proposal.clone(),
        ExitCode::USR_ILLEGAL_ARGUMENT,
    );

    // Same deal with a different label should work
    deal_proposal.label = Label::String("Cthulhu".to_owned());
    rt.set_caller(*ACCOUNT_ACTOR_CODE_ID, WORKER_ADDR);
    let ids = publish_deals(&rt, &miner_addresses, &[deal_proposal], TokenAmount::zero(), 1);
    assert_eq!(1, ids.len());
    check_state(&rt);
}

#[test]
fn max_deal_label_size() {
    let rt = setup();
    let miner_addresses = MinerAddresses {
        owner: OWNER_ADDR,
        worker: WORKER_ADDR,
        provider: PROVIDER_ADDR,
        control: vec![],
    };

    // Test adding provider funds from both worker and owner address
    let funds = TokenAmount::from_atto(20_000_000);
    add_provider_funds(&rt, funds.clone(), &MinerAddresses::default());
    assert_eq!(funds, get_balance(&rt, &PROVIDER_ADDR).balance);

    add_participant_funds(&rt, CLIENT_ADDR, funds);
    let mut deal_proposal =
        generate_deal_proposal(CLIENT_ADDR, PROVIDER_ADDR, 1, 200 * EPOCHS_IN_DAY);

    // DealLabel at max size should work.
    deal_proposal.label = Label::String("s".repeat(DEAL_MAX_LABEL_SIZE));
    rt.set_caller(*ACCOUNT_ACTOR_CODE_ID, WORKER_ADDR);
    let ids =
        publish_deals(&rt, &miner_addresses, &[deal_proposal.clone()], TokenAmount::zero(), 1);
    assert_eq!(1, ids.len());

    // over max should fail
    deal_proposal.label = Label::String("s".repeat(DEAL_MAX_LABEL_SIZE + 1));
    publish_deals_expect_abort(
        &rt,
        &miner_addresses,
        deal_proposal,
        ExitCode::USR_ILLEGAL_ARGUMENT,
    );

    check_state(&rt);
}

#[test]
/// Tests that if 2 deals are published, and the client can't cover collateral for the first deal,
/// but can cover the second, then the first deal fails, but the second passes
fn insufficient_client_balance_in_a_batch() {
    let rt = setup();
    let st: State = rt.get_state();
    let next_deal_id = st.next_id;

    let mut deal1 = generate_deal_proposal(
        CLIENT_ADDR,
        PROVIDER_ADDR,
        ChainEpoch::from(1),
        200 * EPOCHS_IN_DAY,
    );
    let deal2 = generate_deal_proposal(
        CLIENT_ADDR,
        PROVIDER_ADDR,
        ChainEpoch::from(1),
        200 * EPOCHS_IN_DAY,
    );
    deal1.client_collateral = &deal2.client_collateral + TokenAmount::from_atto(1);

    // Client gets enough funds for the 2nd deal
    add_participant_funds(&rt, CLIENT_ADDR, deal2.client_balance_requirement());

    // Provider has enough for both
    let provider_funds =
        deal1.provider_balance_requirement().add(deal2.provider_balance_requirement());
    rt.set_caller(*ACCOUNT_ACTOR_CODE_ID, OWNER_ADDR);
    rt.set_received(provider_funds);
    rt.expect_validate_caller_any();
    expect_provider_control_address(&rt, PROVIDER_ADDR, OWNER_ADDR, WORKER_ADDR);

    assert!(rt
        .call::<MarketActor>(
            Method::AddBalance as u64,
            IpldBlock::serialize_cbor(&PROVIDER_ADDR).unwrap(),
        )
        .unwrap()
        .is_none());

    rt.verify();

    assert_eq!(deal2.client_balance_requirement(), get_balance(&rt, &CLIENT_ADDR).balance);
    assert_eq!(
        deal1.provider_balance_requirement().add(deal2.provider_balance_requirement()),
        get_balance(&rt, &PROVIDER_ADDR).balance
    );

    let buf1 = RawBytes::serialize(&deal1).expect("failed to marshal deal proposal");
    let buf2 = RawBytes::serialize(&deal2).expect("failed to marshal deal proposal");

    let sig1 = Signature::new_bls(buf1.to_vec());
    let sig2 = Signature::new_bls(buf2.to_vec());
    let params = PublishStorageDealsParams {
        deals: vec![
            ClientDealProposal { proposal: deal1.clone(), client_signature: sig1 },
            ClientDealProposal { proposal: deal2.clone(), client_signature: sig2 },
        ],
    };

    rt.expect_validate_caller_any();
    expect_provider_is_control_address(&rt, PROVIDER_ADDR, WORKER_ADDR, true);
    expect_query_network_info(&rt);

    let authenticate_param1 = IpldBlock::serialize_cbor(&AuthenticateMessageParams {
        signature: buf1.to_vec(),
        message: buf1.to_vec(),
    })
    .unwrap();
    let authenticate_param2 = IpldBlock::serialize_cbor(&AuthenticateMessageParams {
        signature: buf2.to_vec(),
        message: buf2.to_vec(),
    })
    .unwrap();

    rt.expect_send(
        deal1.client,
        AUTHENTICATE_MESSAGE_METHOD,
        authenticate_param1,
        TokenAmount::zero(),
        None,
        SendFlags::READ_ONLY,
        AUTHENTICATE_MESSAGE_RESPONSE.clone(),
        ExitCode::OK,
        None,
    );
    rt.expect_send(
        deal2.client,
        AUTHENTICATE_MESSAGE_METHOD,
        authenticate_param2,
        TokenAmount::zero(),
        None,
        SendFlags::READ_ONLY,
        AUTHENTICATE_MESSAGE_RESPONSE.clone(),
        ExitCode::OK,
        None,
    );

    // only valid deals notified
    let notify_param2 = IpldBlock::serialize_cbor(&MarketNotifyDealParams {
        proposal: buf2.to_vec(),
        deal_id: next_deal_id,
    })
    .unwrap();
    rt.expect_send_simple(
        deal2.client,
        MARKET_NOTIFY_DEAL_METHOD,
        notify_param2,
        TokenAmount::zero(),
        None,
        ExitCode::OK,
    );

    expect_emitted(
        &rt,
        "deal-published",
        next_deal_id,
        deal2.client.id().unwrap(),
        deal2.provider.id().unwrap(),
    );

    rt.set_caller(*ACCOUNT_ACTOR_CODE_ID, WORKER_ADDR);

    let ret: PublishStorageDealsReturn = rt
        .call::<MarketActor>(
            Method::PublishStorageDeals as u64,
            IpldBlock::serialize_cbor(&params).unwrap(),
        )
        .unwrap()
        .unwrap()
        .deserialize()
        .unwrap();

    assert!(ret.valid_deals.get(1));
    assert!(!ret.valid_deals.get(0));

    rt.verify();

    check_state(&rt);
}

#[test]
/// Tests that if 2 deals are published, and the provider can't cover collateral for the first deal,
/// but can cover the second, then the first deal fails, but the second passes
fn insufficient_provider_balance_in_a_batch() {
    let rt = setup();
    let st: State = rt.get_state();
    let next_deal_id = st.next_id;

    let mut deal1 = generate_deal_proposal(
        CLIENT_ADDR,
        PROVIDER_ADDR,
        ChainEpoch::from(1),
        200 * EPOCHS_IN_DAY,
    );
    let deal2 = generate_deal_proposal(
        CLIENT_ADDR,
        PROVIDER_ADDR,
        ChainEpoch::from(1),
        200 * EPOCHS_IN_DAY,
    );
    deal1.provider_collateral = &deal2.provider_collateral + TokenAmount::from_atto(1);

    // Client gets enough funds for both deals
    add_participant_funds(
        &rt,
        CLIENT_ADDR,
        deal1.client_balance_requirement().add(deal2.client_balance_requirement()),
    );

    // Provider has enough for only the second deal
    rt.set_caller(*ACCOUNT_ACTOR_CODE_ID, OWNER_ADDR);
    rt.set_received(deal2.provider_balance_requirement().clone());
    rt.expect_validate_caller_any();
    expect_provider_control_address(&rt, PROVIDER_ADDR, OWNER_ADDR, WORKER_ADDR);

    assert!(rt
        .call::<MarketActor>(
            Method::AddBalance as u64,
            IpldBlock::serialize_cbor(&PROVIDER_ADDR).unwrap(),
        )
        .unwrap()
        .is_none());

    rt.verify();

    assert_eq!(
        deal1.client_balance_requirement().add(deal2.client_balance_requirement()),
        get_balance(&rt, &CLIENT_ADDR).balance
    );
    assert_eq!(
        deal2.provider_balance_requirement().clone(),
        get_balance(&rt, &PROVIDER_ADDR).balance
    );

    let buf1 = RawBytes::serialize(&deal1).expect("failed to marshal deal proposal");
    let buf2 = RawBytes::serialize(&deal2).expect("failed to marshal deal proposal");

    let sig1 = Signature::new_bls(buf1.to_vec());
    let sig2 = Signature::new_bls(buf2.to_vec());

    let params = PublishStorageDealsParams {
        deals: vec![
            ClientDealProposal { proposal: deal1.clone(), client_signature: sig1 },
            ClientDealProposal { proposal: deal2.clone(), client_signature: sig2 },
        ],
    };

    rt.expect_validate_caller_any();
    expect_provider_is_control_address(&rt, PROVIDER_ADDR, WORKER_ADDR, true);
    expect_query_network_info(&rt);

    let authenticate_param1 = IpldBlock::serialize_cbor(&AuthenticateMessageParams {
        signature: buf1.to_vec(),
        message: buf1.to_vec(),
    })
    .unwrap();
    let authenticate_param2 = IpldBlock::serialize_cbor(&AuthenticateMessageParams {
        signature: buf2.to_vec(),
        message: buf2.to_vec(),
    })
    .unwrap();

    rt.expect_send(
        deal1.client,
        AUTHENTICATE_MESSAGE_METHOD,
        authenticate_param1,
        TokenAmount::zero(),
        None,
        SendFlags::READ_ONLY,
        AUTHENTICATE_MESSAGE_RESPONSE.clone(),
        ExitCode::OK,
        None,
    );
    rt.expect_send(
        deal2.client,
        AUTHENTICATE_MESSAGE_METHOD,
        authenticate_param2,
        TokenAmount::zero(),
        None,
        SendFlags::READ_ONLY,
        AUTHENTICATE_MESSAGE_RESPONSE.clone(),
        ExitCode::OK,
        None,
    );

    // only valid deal notified
    let notify_param2 = IpldBlock::serialize_cbor(&MarketNotifyDealParams {
        proposal: buf2.to_vec(),
        deal_id: next_deal_id,
    })
    .unwrap();
    rt.expect_send_simple(
        deal2.client,
        MARKET_NOTIFY_DEAL_METHOD,
        notify_param2,
        TokenAmount::zero(),
        None,
        ExitCode::OK,
    );

    rt.set_caller(*ACCOUNT_ACTOR_CODE_ID, WORKER_ADDR);

    expect_emitted(
        &rt,
        "deal-published",
        next_deal_id,
        deal2.client.id().unwrap(),
        deal2.provider.id().unwrap(),
    );

    let ret: PublishStorageDealsReturn = rt
        .call::<MarketActor>(
            Method::PublishStorageDeals as u64,
            IpldBlock::serialize_cbor(&params).unwrap(),
        )
        .unwrap()
        .unwrap()
        .deserialize()
        .unwrap();

    assert!(ret.valid_deals.get(1));
    assert!(!ret.valid_deals.get(0));

    rt.verify();

    check_state(&rt);
}

#[test]
fn add_balance_restricted_correctly() {
    let rt = setup();
    let amount = TokenAmount::from_atto(1000);
    rt.set_received(amount);

    // set caller to not-builtin
    rt.set_caller(*EVM_ACTOR_CODE_ID, Address::new_id(1234));

    // cannot call the unexported method num
    expect_abort_contains_message(
        ExitCode::USR_FORBIDDEN,
        "must be built-in",
        rt.call::<MarketActor>(
            Method::AddBalance as MethodNum,
            IpldBlock::serialize_cbor(&CLIENT_ADDR).unwrap(),
        ),
    );

    // can call the exported method num
    rt.expect_validate_caller_any();
    rt.call::<MarketActor>(
        Method::AddBalanceExported as MethodNum,
        IpldBlock::serialize_cbor(&CLIENT_ADDR).unwrap(),
    )
    .unwrap();

    rt.verify();
}

#[test]
fn psd_restricted_correctly() {
    let rt = setup();
    let st: State = rt.get_state();
    let next_deal_id = st.next_id;

    let deal = generate_deal_proposal(
        CLIENT_ADDR,
        PROVIDER_ADDR,
        ChainEpoch::from(1),
        200 * EPOCHS_IN_DAY,
    );

    // Client gets enough funds
    add_participant_funds(&rt, CLIENT_ADDR, deal.client_balance_requirement());

    // Provider has enough funds
    rt.set_caller(*ACCOUNT_ACTOR_CODE_ID, OWNER_ADDR);
    rt.set_received(deal.provider_balance_requirement().clone());
    rt.expect_validate_caller_any();
    expect_provider_control_address(&rt, PROVIDER_ADDR, OWNER_ADDR, WORKER_ADDR);

    assert!(rt
        .call::<MarketActor>(
            Method::AddBalance as u64,
            IpldBlock::serialize_cbor(&PROVIDER_ADDR).unwrap(),
        )
        .unwrap()
        .is_none());

    rt.verify();

    // Prep the message

    let buf = RawBytes::serialize(&deal).expect("failed to marshal deal proposal");

    let sig = Signature::new_bls(buf.to_vec());

    let params = PublishStorageDealsParams {
        deals: vec![ClientDealProposal { proposal: deal.clone(), client_signature: sig }],
    };

    // set caller to not-builtin
    rt.set_caller(*EVM_ACTOR_CODE_ID, WORKER_ADDR);

    // cannot call the unexported method num
    expect_abort_contains_message(
        ExitCode::USR_FORBIDDEN,
        "must be built-in",
        rt.call::<MarketActor>(
            Method::PublishStorageDeals as MethodNum,
            IpldBlock::serialize_cbor(&params).unwrap(),
        ),
    );

    // can call the exported method num

    let authenticate_param1 = IpldBlock::serialize_cbor(&AuthenticateMessageParams {
        signature: buf.to_vec(),
        message: buf.to_vec(),
    })
    .unwrap();

    rt.expect_validate_caller_any();
    expect_provider_is_control_address(&rt, PROVIDER_ADDR, WORKER_ADDR, true);
    expect_query_network_info(&rt);

    rt.expect_send(
        deal.client,
        AUTHENTICATE_MESSAGE_METHOD,
        authenticate_param1,
        TokenAmount::zero(),
        None,
        SendFlags::READ_ONLY,
        AUTHENTICATE_MESSAGE_RESPONSE.clone(),
        ExitCode::OK,
        None,
    );

    let notify_param = IpldBlock::serialize_cbor(&MarketNotifyDealParams {
        proposal: buf.to_vec(),
        deal_id: next_deal_id,
    })
    .unwrap();
    rt.expect_send_simple(
        deal.client,
        MARKET_NOTIFY_DEAL_METHOD,
        notify_param,
        TokenAmount::zero(),
        None,
        ExitCode::OK,
    );

    expect_emitted(
        &rt,
        "deal-published",
        next_deal_id,
        deal.client.id().unwrap(),
        deal.provider.id().unwrap(),
    );

    let ret: PublishStorageDealsReturn = rt
        .call::<MarketActor>(
            Method::PublishStorageDealsExported as MethodNum,
            IpldBlock::serialize_cbor(&params).unwrap(),
        )
        .unwrap()
        .unwrap()
        .deserialize()
        .unwrap();

    assert!(ret.valid_deals.get(0));

    rt.verify();
    check_state(&rt);
}
