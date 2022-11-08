// Copyright 2019-2022 ChainSafe Systems
// SPDX-License-Identifier: Apache-2.0, MIT

use fil_actor_market_state_v9::balance_table::BALANCE_TABLE_BITWIDTH;
use fil_actor_market_state_v9::policy::detail::DEAL_MAX_LABEL_SIZE;
use fil_actor_market_state_v9::{
    deal_id_key, ext, ActivateDealsParams, Actor as MarketActor, ClientDealProposal, DealArray,
    DealMetaArray, Label, Method, PublishStorageDealsParams, PublishStorageDealsReturn, State,
    WithdrawBalanceParams, NO_ALLOCATION_ID, PROPOSALS_AMT_BITWIDTH, STATES_AMT_BITWIDTH,
};
use fil_actors_runtime_common::cbor::{deserialize, serialize};
use fil_actors_runtime_common::network::EPOCHS_IN_DAY;
use fil_actors_runtime_common::runtime::{builtins::Type, Policy, Runtime};
use fil_actors_runtime_common::test_utils::*;
use fil_actors_runtime_common::{
    make_empty_map, make_map_with_root_and_bitwidth, ActorError, BatchReturn, Map, SetMultimap,
    BURNT_FUNDS_ACTOR_ADDR, CALLER_TYPES_SIGNABLE, DATACAP_TOKEN_ACTOR_ADDR, SYSTEM_ACTOR_ADDR,
    VERIFIED_REGISTRY_ACTOR_ADDR,
};
use frc46_token::token::types::{TransferFromParams, TransferFromReturn};
use fvm_ipld_amt::Amt;
use fvm_ipld_encoding::{to_vec, RawBytes};
use fvm_shared::address::Address;
use fvm_shared::clock::{ChainEpoch, EPOCH_UNDEFINED};
use fvm_shared::crypto::signature::Signature;
use fvm_shared::deal::DealID;
use fvm_shared::econ::TokenAmount;
use fvm_shared::error::ExitCode;
use fvm_shared::piece::PaddedPieceSize;
use fvm_shared::sector::StoragePower;
use fvm_shared::{HAMT_BIT_WIDTH, METHOD_CONSTRUCTOR, METHOD_SEND};
use regex::Regex;
use std::ops::Add;

use fil_actor_market_state_v9::ext::account::{
    AuthenticateMessageParams, AUTHENTICATE_MESSAGE_METHOD,
};
use fil_actor_market_state_v9::ext::verifreg::{
    AllocationID, AllocationRequest, AllocationsResponse,
};
use num_traits::{FromPrimitive, Zero};

mod harness;

use harness::*;

#[test]
fn test_remove_all_error() {
    let market_actor = Address::new_id(100);
    let rt = MockRuntime { receiver: market_actor, ..Default::default() };

    SetMultimap::new(&rt.store()).remove_all(42).expect("expected no error");
}

// TODO add array stuff
#[test]
fn simple_construction() {
    let mut rt = MockRuntime {
        receiver: Address::new_id(100),
        caller: SYSTEM_ACTOR_ADDR,
        caller_type: *INIT_ACTOR_CODE_ID,
        ..Default::default()
    };

    rt.expect_validate_caller_addr(vec![SYSTEM_ACTOR_ADDR]);

    assert_eq!(
        RawBytes::default(),
        rt.call::<MarketActor>(METHOD_CONSTRUCTOR, &RawBytes::default()).unwrap()
    );

    rt.verify();

    let store = &rt.store;

    let empty_balance_table =
        make_empty_map::<_, TokenAmount>(store, BALANCE_TABLE_BITWIDTH).flush().unwrap();
    let empty_map = make_empty_map::<_, ()>(store, HAMT_BIT_WIDTH).flush().unwrap();
    let empty_proposals_array =
        Amt::<(), _>::new_with_bit_width(store, PROPOSALS_AMT_BITWIDTH).flush().unwrap();
    let empty_states_array =
        Amt::<(), _>::new_with_bit_width(store, STATES_AMT_BITWIDTH).flush().unwrap();
    let empty_multimap = SetMultimap::new(store).root().unwrap();

    let state_data: State = rt.get_state();

    assert_eq!(empty_proposals_array, state_data.proposals);
    assert_eq!(empty_states_array, state_data.states);
    assert_eq!(empty_map, state_data.pending_proposals);
    assert_eq!(empty_balance_table, state_data.escrow_table);
    assert_eq!(empty_balance_table, state_data.locked_table);
    assert_eq!(0, state_data.next_id);
    assert_eq!(empty_multimap, state_data.deal_ops_by_epoch);
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
        let mut rt = setup();

        for tc in &test_cases {
            rt.set_caller(*ACCOUNT_ACTOR_CODE_ID, *caller_addr);
            rt.set_value(TokenAmount::from_atto(tc.delta));
            rt.expect_validate_caller_type((*CALLER_TYPES_SIGNABLE).to_vec());
            expect_provider_control_address(&mut rt, PROVIDER_ADDR, OWNER_ADDR, WORKER_ADDR);

            assert_eq!(
                RawBytes::default(),
                rt.call::<MarketActor>(
                    Method::AddBalance as u64,
                    &RawBytes::serialize(PROVIDER_ADDR).unwrap(),
                )
                .unwrap()
            );

            rt.verify();

            assert_eq!(
                get_escrow_balance(&rt, &PROVIDER_ADDR).unwrap(),
                TokenAmount::from_atto(tc.total)
            );
            check_state(&rt);
        }
    }
}

#[test]
fn fails_if_withdraw_from_non_provider_funds_is_not_initiated_by_the_recipient() {
    let mut rt = setup();

    add_participant_funds(&mut rt, CLIENT_ADDR, TokenAmount::from_atto(20u8));

    assert_eq!(TokenAmount::from_atto(20u8), get_escrow_balance(&rt, &CLIENT_ADDR).unwrap());

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
            &RawBytes::serialize(params).unwrap(),
        ),
    );
    rt.verify();

    // verify there was no withdrawal
    assert_eq!(TokenAmount::from_atto(20u8), get_escrow_balance(&rt, &CLIENT_ADDR).unwrap());

    check_state(&rt);
}

#[test]
fn balance_after_withdrawal_must_always_be_greater_than_or_equal_to_locked_amount() {
    let start_epoch = ChainEpoch::from(10);
    let end_epoch = start_epoch + 200 * EPOCHS_IN_DAY;
    let publish_epoch = ChainEpoch::from(5);

    let mut rt = setup();

    // publish the deal so that client AND provider collateral is locked
    rt.set_epoch(publish_epoch);
    let deal_id = generate_and_publish_deal(
        &mut rt,
        CLIENT_ADDR,
        &MinerAddresses::default(),
        start_epoch,
        end_epoch,
    );
    let deal = get_deal_proposal(&mut rt, deal_id);
    assert_eq!(deal.provider_collateral, get_escrow_balance(&rt, &PROVIDER_ADDR).unwrap());
    assert_eq!(deal.client_balance_requirement(), get_escrow_balance(&rt, &CLIENT_ADDR).unwrap());

    let withdraw_amount = TokenAmount::from_atto(1u8);
    let withdrawable_amount = TokenAmount::zero();
    // client cannot withdraw any funds since all it's balance is locked
    withdraw_client_balance(
        &mut rt,
        withdraw_amount.clone(),
        withdrawable_amount.clone(),
        CLIENT_ADDR,
    );
    // provider cannot withdraw any funds since all it's balance is locked
    withdraw_provider_balance(
        &mut rt,
        withdraw_amount,
        withdrawable_amount,
        PROVIDER_ADDR,
        OWNER_ADDR,
        WORKER_ADDR,
    );

    // add some more funds to the provider & ensure withdrawal is limited by the locked funds
    let withdraw_amount = TokenAmount::from_atto(30u8);
    let withdrawable_amount = TokenAmount::from_atto(25u8);

    add_provider_funds(&mut rt, withdrawable_amount.clone(), &MinerAddresses::default());
    withdraw_provider_balance(
        &mut rt,
        withdraw_amount.clone(),
        withdrawable_amount.clone(),
        PROVIDER_ADDR,
        OWNER_ADDR,
        WORKER_ADDR,
    );

    // add some more funds to the client & ensure withdrawal is limited by the locked funds
    add_participant_funds(&mut rt, CLIENT_ADDR, withdrawable_amount.clone());
    withdraw_client_balance(&mut rt, withdraw_amount, withdrawable_amount, CLIENT_ADDR);
    check_state(&rt);
}

#[test]
fn worker_balance_after_withdrawal_must_account_for_slashed_funds() {
    let start_epoch = ChainEpoch::from(10);
    let end_epoch = start_epoch + 200 * EPOCHS_IN_DAY;
    let publish_epoch = ChainEpoch::from(5);

    let mut rt = setup();

    // publish deal
    rt.set_epoch(publish_epoch);
    let deal_id = generate_and_publish_deal(
        &mut rt,
        CLIENT_ADDR,
        &MinerAddresses::default(),
        start_epoch,
        end_epoch,
    );

    // activate the deal
    activate_deals(&mut rt, end_epoch + 1, PROVIDER_ADDR, publish_epoch, &[deal_id]);
    let st = get_deal_state(&mut rt, deal_id);
    assert_eq!(publish_epoch, st.sector_start_epoch);

    // slash the deal
    rt.set_epoch(publish_epoch + 1);
    terminate_deals(&mut rt, PROVIDER_ADDR, &[deal_id]);
    let st = get_deal_state(&mut rt, deal_id);
    assert_eq!(publish_epoch + 1, st.slash_epoch);

    // provider cannot withdraw any funds since all it's balance is locked
    let withdraw_amount = TokenAmount::from_atto(1);
    let actual_withdrawn = TokenAmount::zero();
    withdraw_provider_balance(
        &mut rt,
        withdraw_amount,
        actual_withdrawn,
        PROVIDER_ADDR,
        OWNER_ADDR,
        WORKER_ADDR,
    );

    // add some more funds to the provider & ensure withdrawal is limited by the locked funds
    add_provider_funds(&mut rt, TokenAmount::from_atto(25), &MinerAddresses::default());
    let withdraw_amount = TokenAmount::from_atto(30);
    let actual_withdrawn = TokenAmount::from_atto(25);

    withdraw_provider_balance(
        &mut rt,
        withdraw_amount,
        actual_withdrawn,
        PROVIDER_ADDR,
        OWNER_ADDR,
        WORKER_ADDR,
    );
    check_state(&rt);
}

#[test]
fn fails_unless_called_by_an_account_actor() {
    let mut rt = setup();

    rt.set_value(TokenAmount::from_atto(10));
    rt.expect_validate_caller_type((*CALLER_TYPES_SIGNABLE).to_vec());

    rt.set_caller(*MINER_ACTOR_CODE_ID, PROVIDER_ADDR);
    assert_eq!(
        ExitCode::USR_FORBIDDEN,
        rt.call::<MarketActor>(
            Method::AddBalance as u64,
            &RawBytes::serialize(PROVIDER_ADDR).unwrap(),
        )
        .unwrap_err()
        .exit_code()
    );

    rt.verify();
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
        let mut rt = setup();

        for tc in &test_cases {
            rt.set_caller(*ACCOUNT_ACTOR_CODE_ID, *caller_addr);
            rt.set_value(TokenAmount::from_atto(tc.delta));
            rt.expect_validate_caller_type((*CALLER_TYPES_SIGNABLE).to_vec());

            assert_eq!(
                RawBytes::default(),
                rt.call::<MarketActor>(
                    Method::AddBalance as u64,
                    &RawBytes::serialize(caller_addr).unwrap(),
                )
                .unwrap()
            );

            rt.verify();

            assert_eq!(
                get_escrow_balance(&rt, caller_addr).unwrap(),
                TokenAmount::from_atto(tc.total)
            );
            check_state(&rt);
        }
    }
}

#[test]
fn withdraws_from_provider_escrow_funds_and_sends_to_owner() {
    let mut rt = setup();

    let amount = TokenAmount::from_atto(20);
    add_provider_funds(&mut rt, amount.clone(), &MinerAddresses::default());

    assert_eq!(amount, get_escrow_balance(&rt, &PROVIDER_ADDR).unwrap());

    // worker calls WithdrawBalance, balance is transferred to owner
    let withdraw_amount = TokenAmount::from_atto(1);
    withdraw_provider_balance(
        &mut rt,
        withdraw_amount.clone(),
        withdraw_amount,
        PROVIDER_ADDR,
        OWNER_ADDR,
        WORKER_ADDR,
    );

    assert_eq!(TokenAmount::from_atto(19), get_escrow_balance(&rt, &PROVIDER_ADDR).unwrap());
    check_state(&rt);
}

#[test]
fn withdraws_from_non_provider_escrow_funds() {
    let mut rt = setup();

    let amount = TokenAmount::from_atto(20);
    add_participant_funds(&mut rt, CLIENT_ADDR, amount.clone());

    assert_eq!(get_escrow_balance(&rt, &CLIENT_ADDR).unwrap(), amount);

    let withdraw_amount = TokenAmount::from_atto(1);
    withdraw_client_balance(&mut rt, withdraw_amount.clone(), withdraw_amount, CLIENT_ADDR);

    assert_eq!(get_escrow_balance(&rt, &CLIENT_ADDR).unwrap(), TokenAmount::from_atto(19));
    check_state(&rt);
}

#[test]
fn client_withdrawing_more_than_escrow_balance_limits_to_available_funds() {
    let mut rt = setup();

    let amount = TokenAmount::from_atto(20);
    add_participant_funds(&mut rt, CLIENT_ADDR, amount.clone());

    // withdraw amount greater than escrow balance
    let withdraw_amount = TokenAmount::from_atto(25);
    withdraw_client_balance(&mut rt, withdraw_amount, amount, CLIENT_ADDR);

    assert_eq!(get_escrow_balance(&rt, &CLIENT_ADDR).unwrap(), TokenAmount::zero());
    check_state(&rt);
}

#[test]
fn worker_withdrawing_more_than_escrow_balance_limits_to_available_funds() {
    let mut rt = setup();

    let amount = TokenAmount::from_atto(20);
    add_provider_funds(&mut rt, amount.clone(), &MinerAddresses::default());

    assert_eq!(get_escrow_balance(&rt, &PROVIDER_ADDR).unwrap(), amount);

    let withdraw_amount = TokenAmount::from_atto(25);
    withdraw_provider_balance(
        &mut rt,
        withdraw_amount,
        amount,
        PROVIDER_ADDR,
        OWNER_ADDR,
        WORKER_ADDR,
    );

    assert_eq!(get_escrow_balance(&rt, &PROVIDER_ADDR).unwrap(), TokenAmount::zero());
    check_state(&rt);
}

#[test]
fn fail_when_balance_is_zero() {
    let mut rt = setup();

    rt.set_caller(*ACCOUNT_ACTOR_CODE_ID, OWNER_ADDR);
    rt.set_received(TokenAmount::zero());

    expect_abort(
        ExitCode::USR_ILLEGAL_ARGUMENT,
        rt.call::<MarketActor>(
            Method::AddBalance as u64,
            &RawBytes::serialize(&PROVIDER_ADDR).unwrap(),
        ),
    );

    rt.verify();
    check_state(&rt);
}

#[test]
fn fails_with_a_negative_withdraw_amount() {
    let mut rt = setup();

    let params = WithdrawBalanceParams {
        provider_or_client: PROVIDER_ADDR,
        amount: TokenAmount::from_atto(-1_i32),
    };

    expect_abort(
        ExitCode::USR_ILLEGAL_ARGUMENT,
        rt.call::<MarketActor>(
            Method::WithdrawBalance as u64,
            &RawBytes::serialize(&params).unwrap(),
        ),
    );

    rt.verify();
    check_state(&rt);
}

#[test]
fn fails_if_withdraw_from_provider_funds_is_not_initiated_by_the_owner_or_worker() {
    let mut rt = setup();

    let amount = TokenAmount::from_atto(20u8);
    add_provider_funds(&mut rt, amount.clone(), &MinerAddresses::default());

    assert_eq!(get_escrow_balance(&rt, &PROVIDER_ADDR).unwrap(), amount);

    // only signing parties can add balance for client AND provider.
    rt.expect_validate_caller_addr(vec![OWNER_ADDR, WORKER_ADDR]);
    let params = WithdrawBalanceParams {
        provider_or_client: PROVIDER_ADDR,
        amount: TokenAmount::from_atto(1u8),
    };

    // caller is not owner or worker
    rt.set_caller(*ACCOUNT_ACTOR_CODE_ID, Address::new_id(909));
    expect_provider_control_address(&mut rt, PROVIDER_ADDR, OWNER_ADDR, WORKER_ADDR);

    expect_abort(
        ExitCode::USR_FORBIDDEN,
        rt.call::<MarketActor>(
            Method::WithdrawBalance as u64,
            &RawBytes::serialize(&params).unwrap(),
        ),
    );
    rt.verify();

    // verify there was no withdrawal
    assert_eq!(get_escrow_balance(&rt, &PROVIDER_ADDR).unwrap(), amount);
    check_state(&rt);
}

#[test]
fn deal_starts_on_day_boundary() {
    let deal_updates_interval = Policy::default().deal_updates_interval;
    let start_epoch = deal_updates_interval; // 2880
    let end_epoch = start_epoch + 200 * EPOCHS_IN_DAY;
    let publish_epoch = ChainEpoch::from(1);

    let mut rt = setup();
    rt.set_epoch(publish_epoch);

    for i in 0..(3 * deal_updates_interval) {
        let piece_cid = make_piece_cid((format!("{i}")).as_bytes());
        let deal_id = generate_and_publish_deal_for_piece(
            &mut rt,
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
    let dobe = SetMultimap::from_root(store, &st.deal_ops_by_epoch).unwrap();
    for e in deal_updates_interval..(2 * deal_updates_interval) {
        assert_n_good_deals(&dobe, e, 3);
    }

    // DOBE has no deals scheduled in the previous or next day
    for e in 0..deal_updates_interval {
        assert_n_good_deals(&dobe, e, 0);
    }
    for e in (2 * deal_updates_interval)..(3 * deal_updates_interval) {
        assert_n_good_deals(&dobe, e, 0);
    }
}

#[test]
fn deal_starts_partway_through_day() {
    let start_epoch = 1000;
    let end_epoch = start_epoch + 200 * EPOCHS_IN_DAY;
    let publish_epoch = ChainEpoch::from(1);

    let mut rt = setup();
    rt.set_epoch(publish_epoch);

    // First 1000 deals (start_epoch % update interval) scheduled starting in the next day
    for i in 0..1000 {
        let piece_cid = make_piece_cid((format!("{i}")).as_bytes());
        let deal_id = generate_and_publish_deal_for_piece(
            &mut rt,
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
    let dobe = SetMultimap::from_root(store, &st.deal_ops_by_epoch).unwrap();
    for e in 2880..(2880 + start_epoch) {
        assert_n_good_deals(&dobe, e, 1);
    }
    // Nothing scheduled between 0 and 2880
    for e in 0..2880 {
        assert_n_good_deals(&dobe, e, 0);
    }

    // Now add another 500 deals
    for i in 1000..1500 {
        let piece_cid = make_piece_cid((format!("{i}")).as_bytes());
        let deal_id = generate_and_publish_deal_for_piece(
            &mut rt,
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
    let dobe = SetMultimap::from_root(store, &st.deal_ops_by_epoch).unwrap();
    for e in start_epoch..(start_epoch + 500) {
        assert_n_good_deals(&dobe, e, 1);
    }
}

#[test]
fn simple_deal() {
    let start_epoch = 1000;
    let end_epoch = start_epoch + 200 * EPOCHS_IN_DAY;
    let publish_epoch = ChainEpoch::from(1);

    let mut rt = setup();
    rt.set_epoch(publish_epoch);
    let next_allocation_id = 1;

    // Publish from miner worker.
    let mut deal1 = generate_deal_and_add_funds(
        &mut rt,
        CLIENT_ADDR,
        &MinerAddresses::default(),
        start_epoch,
        end_epoch,
    );
    deal1.verified_deal = false;
    rt.set_caller(*ACCOUNT_ACTOR_CODE_ID, WORKER_ADDR);
    let deal1_id =
        publish_deals(&mut rt, &MinerAddresses::default(), &[deal1], next_allocation_id)[0];

    // Publish from miner control address.
    let mut deal2 = generate_deal_and_add_funds(
        &mut rt,
        CLIENT_ADDR,
        &MinerAddresses::default(),
        start_epoch + 1,
        end_epoch + 1,
    );
    deal2.verified_deal = true;
    rt.set_caller(*ACCOUNT_ACTOR_CODE_ID, CONTROL_ADDR);
    let deal2_id =
        publish_deals(&mut rt, &MinerAddresses::default(), &[deal2], next_allocation_id)[0];

    // activate the deal
    activate_deals(&mut rt, end_epoch + 1, PROVIDER_ADDR, publish_epoch, &[deal1_id, deal2_id]);
    let deal1st = get_deal_state(&mut rt, deal1_id);
    assert_eq!(publish_epoch, deal1st.sector_start_epoch);
    assert_eq!(NO_ALLOCATION_ID, deal1st.verified_claim);

    let deal2st = get_deal_state(&mut rt, deal2_id);
    assert_eq!(publish_epoch, deal2st.sector_start_epoch);
    assert_eq!(next_allocation_id, deal2st.verified_claim);

    check_state(&rt);
}

#[test]
fn deal_expires() {
    let start_epoch = 100;
    let end_epoch = start_epoch + 200 * EPOCHS_IN_DAY;
    let publish_epoch = ChainEpoch::from(1);

    let mut rt = setup();
    rt.set_epoch(publish_epoch);
    let next_allocation_id = 1;

    // Publish from miner worker.
    let mut deal = generate_deal_and_add_funds(
        &mut rt,
        CLIENT_ADDR,
        &MinerAddresses::default(),
        start_epoch,
        end_epoch,
    );
    deal.verified_deal = true;
    rt.set_caller(*ACCOUNT_ACTOR_CODE_ID, WORKER_ADDR);
    let deal_id =
        publish_deals(&mut rt, &MinerAddresses::default(), &[deal.clone()], next_allocation_id)[0];

    rt.set_epoch(start_epoch + EPOCHS_IN_DAY + 1);
    rt.expect_send(
        BURNT_FUNDS_ACTOR_ADDR,
        METHOD_SEND,
        RawBytes::default(),
        deal.provider_collateral,
        RawBytes::default(),
        ExitCode::OK,
    );
    cron_tick(&mut rt);

    // No deal state for unactivated deal
    let st: State = rt.get_state();
    let states = DealMetaArray::load(&st.states, &rt.store).unwrap();
    assert!(states.get(deal_id).unwrap().is_none());

    // The proposal is gone
    assert!(DealArray::load(&st.proposals, &rt.store).unwrap().get(deal_id).unwrap().is_none());

    // Pending allocation ID is gone
    let pending_allocs: Map<_, AllocationID> =
        make_map_with_root_and_bitwidth(&st.pending_deal_allocation_ids, &rt.store, HAMT_BIT_WIDTH)
            .unwrap();
    assert!(pending_allocs.get(&deal_id_key(deal_id)).unwrap().is_none());

    check_state(&rt);
}

// Converted from: https://github.com/filecoin-project/specs-actors/blob/0afe155bfffa036057af5519afdead845e0780de/actors/builtin/market/market_test.go#L529
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

    let mut rt = setup();
    rt.actor_code_cids.insert(client_resolved, *ACCOUNT_ACTOR_CODE_ID);
    rt.actor_code_cids.insert(provider_resolved, *MINER_ACTOR_CODE_ID);

    // mappings for resolving address
    rt.id_addresses.insert(client_bls, client_resolved);
    rt.id_addresses.insert(provider_bls, provider_resolved);

    // generate deal and add required funds for deal
    let start_epoch = 42;
    let end_epoch = start_epoch + 200 * EPOCHS_IN_DAY;
    rt.set_epoch(start_epoch);

    let mut deal = generate_deal_proposal(client_bls, provider_bls, start_epoch, end_epoch);
    deal.verified_deal = true;

    // add funds for client using its BLS address -> will be resolved and persisted
    add_participant_funds(&mut rt, client_bls, deal.client_balance_requirement());
    assert_eq!(
        deal.client_balance_requirement(),
        get_escrow_balance(&rt, &client_resolved).unwrap()
    );

    // add funds for provider using it's BLS address -> will be resolved and persisted
    rt.value_received = deal.provider_collateral.clone();
    rt.set_caller(*ACCOUNT_ACTOR_CODE_ID, OWNER_ADDR);
    rt.expect_validate_caller_type((*CALLER_TYPES_SIGNABLE).to_vec());
    expect_provider_control_address(&mut rt, provider_resolved, OWNER_ADDR, WORKER_ADDR);

    assert_eq!(
        RawBytes::default(),
        rt.call::<MarketActor>(
            Method::AddBalance as u64,
            &RawBytes::serialize(provider_bls).unwrap(),
        )
        .unwrap()
    );
    rt.verify();
    rt.add_balance(deal.provider_collateral.clone());
    assert_eq!(deal.provider_collateral, get_escrow_balance(&rt, &provider_resolved).unwrap());

    // publish deal using the BLS addresses
    rt.set_caller(*ACCOUNT_ACTOR_CODE_ID, WORKER_ADDR);
    rt.expect_validate_caller_type((*CALLER_TYPES_SIGNABLE).to_vec());

    expect_provider_control_address(&mut rt, provider_resolved, OWNER_ADDR, WORKER_ADDR);
    expect_query_network_info(&mut rt);

    //  create a client proposal with a valid signature
    let mut params = PublishStorageDealsParams { deals: vec![] };
    let buf = RawBytes::serialize(&deal).expect("failed to marshal deal proposal");
    let sig = Signature::new_bls(buf.to_vec());
    let client_proposal = ClientDealProposal { client_signature: sig, proposal: deal.clone() };
    params.deals.push(client_proposal);
    // expect a call to verify the above signature

    let auth_param = RawBytes::serialize(AuthenticateMessageParams {
        signature: buf.to_vec(),
        message: buf.to_vec(),
    })
    .unwrap();

    rt.expect_send(
        deal.client,
        AUTHENTICATE_MESSAGE_METHOD,
        auth_param,
        TokenAmount::zero(),
        RawBytes::default(),
        ExitCode::OK,
    );

    // Data cap transfer is requested using the resolved address (not that it matters).
    let alloc_req = ext::verifreg::AllocationRequests {
        allocations: vec![AllocationRequest {
            provider: provider_resolved,
            data: deal.piece_cid,
            size: deal.piece_size,
            term_min: deal.end_epoch - deal.start_epoch,
            term_max: (deal.end_epoch - deal.start_epoch) + 90 * EPOCHS_IN_DAY,
            expiration: deal.start_epoch,
        }],
        extensions: vec![],
    };
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
    rt.expect_send(
        DATACAP_TOKEN_ACTOR_ADDR,
        ext::datacap::TRANSFER_FROM_METHOD as u64,
        serialize(&transfer_params, "transfer from params").unwrap(),
        TokenAmount::zero(),
        serialize(&transfer_return, "transfer from return").unwrap(),
        ExitCode::OK,
    );

    let ret: PublishStorageDealsReturn = rt
        .call::<MarketActor>(
            Method::PublishStorageDeals as u64,
            &RawBytes::serialize(params).unwrap(),
        )
        .unwrap()
        .deserialize()
        .unwrap();
    rt.verify();
    let deal_id = ret.ids[0];

    // assert that deal is persisted with the resolved addresses
    let prop = get_deal_proposal(&mut rt, deal_id);
    assert_eq!(client_resolved, prop.client);
    assert_eq!(provider_resolved, prop.provider);

    check_state(&rt);
}

#[test]
fn publish_a_deal_after_activating_a_previous_deal_which_has_a_start_epoch_far_in_the_future() {
    let start_epoch = 1000;
    let end_epoch = start_epoch + 200 * EPOCHS_IN_DAY;
    let publish_epoch = ChainEpoch::from(1);

    let mut rt = setup();

    // publish the deal and activate it
    rt.set_epoch(publish_epoch);
    let deal1 = generate_and_publish_deal(
        &mut rt,
        CLIENT_ADDR,
        &MinerAddresses::default(),
        start_epoch,
        end_epoch,
    );
    activate_deals(&mut rt, end_epoch, PROVIDER_ADDR, publish_epoch, &[deal1]);
    let st = get_deal_state(&mut rt, deal1);
    assert_eq!(publish_epoch, st.sector_start_epoch);

    // now publish a second deal and activate it
    let new_epoch = publish_epoch + 1;
    rt.set_epoch(new_epoch);
    let deal2 = generate_and_publish_deal(
        &mut rt,
        CLIENT_ADDR,
        &MinerAddresses::default(),
        start_epoch + 1,
        end_epoch + 1,
    );
    activate_deals(&mut rt, end_epoch + 1, PROVIDER_ADDR, new_epoch, &[deal2]);
    check_state(&rt);
}

#[test]
fn publish_a_deal_with_enough_collateral_when_circulating_supply_is_superior_to_zero() {
    let policy = Policy::default();

    let start_epoch = 1000;
    let end_epoch = start_epoch + 200 * EPOCHS_IN_DAY;
    let publish_epoch = ChainEpoch::from(1);

    let mut rt = setup();

    let client_collateral = TokenAmount::from_atto(10u8); // min is zero so this is placeholder

    // given power and circ supply cancel this should be 1*dealqapower / 100
    let deal_size = PaddedPieceSize(2048u64); // generateDealProposal's deal size
    let provider_collateral = TokenAmount::from_atto(
        (deal_size.0 * (policy.prov_collateral_percent_supply_num as u64))
            / policy.prov_collateral_percent_supply_denom as u64,
    );

    let deal = generate_deal_with_collateral_and_add_funds(
        &mut rt,
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
    publish_deals(&mut rt, &MinerAddresses::default(), &[deal], 1);
    check_state(&rt);
}

#[test]
fn publish_multiple_deals_for_different_clients_and_ensure_balances_are_correct() {
    let start_epoch = 42;
    let end_epoch = start_epoch + 200 * EPOCHS_IN_DAY;

    let mut rt = setup();

    let client1_addr = Address::new_id(900);
    let client2_addr = Address::new_id(901);
    let client3_addr = Address::new_id(902);

    // generate first deal for
    let deal1 = generate_deal_and_add_funds(
        &mut rt,
        client1_addr,
        &MinerAddresses::default(),
        start_epoch,
        end_epoch,
    );

    // generate second deal
    let deal2 = generate_deal_and_add_funds(
        &mut rt,
        client2_addr,
        &MinerAddresses::default(),
        start_epoch,
        end_epoch,
    );

    // generate third deal
    let deal3 = generate_deal_and_add_funds(
        &mut rt,
        client3_addr,
        &MinerAddresses::default(),
        start_epoch,
        end_epoch,
    );

    rt.set_caller(*ACCOUNT_ACTOR_CODE_ID, WORKER_ADDR);
    publish_deals(
        &mut rt,
        &MinerAddresses::default(),
        &[deal1.clone(), deal2.clone(), deal3.clone()],
        1,
    );

    // assert locked balance for all clients and provider
    let provider_locked_expected =
        &deal1.provider_collateral + &deal2.provider_collateral + &deal3.provider_collateral;
    let client1_locked = get_locked_balance(&mut rt, client1_addr);
    let client2_locked = get_locked_balance(&mut rt, client2_addr);
    let client3_locked = get_locked_balance(&mut rt, client3_addr);
    assert_eq!(deal1.client_balance_requirement(), client1_locked);
    assert_eq!(deal2.client_balance_requirement(), client2_locked);
    assert_eq!(deal3.client_balance_requirement(), client3_locked);
    assert_eq!(provider_locked_expected, get_locked_balance(&mut rt, PROVIDER_ADDR));

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
        &mut rt,
        client3_addr,
        &MinerAddresses::default(),
        1000,
        1000 + 200 * EPOCHS_IN_DAY,
    );
    let deal5 = generate_deal_and_add_funds(
        &mut rt,
        client3_addr,
        &MinerAddresses::default(),
        100,
        100 + 200 * EPOCHS_IN_DAY,
    );
    rt.set_caller(*ACCOUNT_ACTOR_CODE_ID, WORKER_ADDR);
    publish_deals(&mut rt, &MinerAddresses::default(), &[deal4.clone(), deal5.clone()], 1);

    // assert locked balances for clients and provider
    let provider_locked_expected =
        &provider_locked_expected + &deal4.provider_collateral + &deal5.provider_collateral;
    assert_eq!(provider_locked_expected, get_locked_balance(&mut rt, PROVIDER_ADDR));

    let client3_locked_updated = get_locked_balance(&mut rt, client3_addr);
    assert_eq!(
        &client3_locked + &deal4.client_balance_requirement() + &deal5.client_balance_requirement(),
        client3_locked_updated
    );

    let client1_locked = get_locked_balance(&mut rt, client1_addr);
    let client2_locked = get_locked_balance(&mut rt, client2_addr);
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
        generate_deal_and_add_funds(&mut rt, client1_addr, &addrs, 20, 20 + 200 * EPOCHS_IN_DAY);

    // generate second deal for second provider
    let deal7 =
        generate_deal_and_add_funds(&mut rt, client1_addr, &addrs, 25, 60 + 200 * EPOCHS_IN_DAY);

    // publish both the deals for the second provider
    rt.set_caller(*ACCOUNT_ACTOR_CODE_ID, WORKER_ADDR);
    publish_deals(&mut rt, &addrs, &[deal6.clone(), deal7.clone()], 1);

    // assertions
    let st: State = rt.get_state();
    let provider2_locked = &deal6.provider_collateral + &deal7.provider_collateral;
    assert_eq!(provider2_locked, get_locked_balance(&mut rt, provider2_addr));
    let client1_locked_updated = get_locked_balance(&mut rt, client1_addr);
    assert_eq!(
        &deal7.client_balance_requirement() + &client1_locked + &deal6.client_balance_requirement(),
        client1_locked_updated
    );

    // assert first provider's balance as well
    assert_eq!(provider_locked_expected, get_locked_balance(&mut rt, PROVIDER_ADDR));

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

    let mut rt = setup();
    rt.set_epoch(current_epoch);

    // provider 1 publishes deals1 and deals2 and deal3
    let deal1 = generate_and_publish_deal(
        &mut rt,
        CLIENT_ADDR,
        &MinerAddresses::default(),
        start_epoch,
        end_epoch,
    );
    let deal2 = generate_and_publish_deal(
        &mut rt,
        CLIENT_ADDR,
        &MinerAddresses::default(),
        start_epoch,
        end_epoch + 1,
    );
    let deal3 = generate_and_publish_deal(
        &mut rt,
        CLIENT_ADDR,
        &MinerAddresses::default(),
        start_epoch,
        end_epoch + 2,
    );

    // provider2 publishes deal4 and deal5
    let provider2_addr = Address::new_id(401);
    let addrs = MinerAddresses { provider: provider2_addr, ..MinerAddresses::default() };
    let deal4 = generate_and_publish_deal(&mut rt, CLIENT_ADDR, &addrs, start_epoch, end_epoch);
    let deal5 = generate_and_publish_deal(&mut rt, CLIENT_ADDR, &addrs, start_epoch, end_epoch + 1);

    // provider1 activates deal1 and deal2 but that does not activate deal3 to deal5
    activate_deals(&mut rt, sector_expiry, PROVIDER_ADDR, current_epoch, &[deal1, deal2]);
    assert_deals_not_activated(&mut rt, current_epoch, &[deal3, deal4, deal5]);

    // provider2 activates deal5 but that does not activate deal3 or deal4
    activate_deals(&mut rt, sector_expiry, provider2_addr, current_epoch, &[deal5]);
    assert_deals_not_activated(&mut rt, current_epoch, &[deal3, deal4]);

    // provider1 activates deal3
    activate_deals(&mut rt, sector_expiry, PROVIDER_ADDR, current_epoch, &[deal3]);
    assert_deals_not_activated(&mut rt, current_epoch, &[deal4]);
    check_state(&rt);
}

// Converted from: https://github.com/filecoin-project/specs-actors/blob/master/actors/builtin/market/market_test.go#L1519
#[test]
fn fail_when_deal_is_activated_but_proposal_is_not_found() {
    let start_epoch = 50;
    let end_epoch = start_epoch + 200 * EPOCHS_IN_DAY;
    let sector_expiry = end_epoch + 100;

    let mut rt = setup();

    let deal_id = publish_and_activate_deal(
        &mut rt,
        CLIENT_ADDR,
        &MinerAddresses::default(),
        start_epoch,
        end_epoch,
        0,
        sector_expiry,
    );

    // delete the deal proposal (this breaks state invariants)
    delete_deal_proposal(&mut rt, deal_id);

    rt.set_epoch(process_epoch(start_epoch, deal_id));
    expect_abort(ExitCode::USR_NOT_FOUND, cron_tick_raw(&mut rt));

    check_state_with_expected(
        &rt,
        &[
            Regex::new("no deal proposal for deal state \\d+").unwrap(),
            Regex::new("pending proposal with cid \\w+ not found within proposals .*").unwrap(),
            Regex::new("deal op found for deal id \\d+ with missing proposal at epoch \\d+")
                .unwrap(),
        ],
    );
}

// Converted from: https://github.com/filecoin-project/specs-actors/blob/master/actors/builtin/market/market_test.go#L1540
#[test]
fn fail_when_deal_update_epoch_is_in_the_future() {
    let start_epoch = 50;
    let end_epoch = start_epoch + 200 * EPOCHS_IN_DAY;
    let sector_expiry = end_epoch + 100;

    let mut rt = setup();

    let deal_id = publish_and_activate_deal(
        &mut rt,
        CLIENT_ADDR,
        &MinerAddresses::default(),
        start_epoch,
        end_epoch,
        0,
        sector_expiry,
    );

    // move the current epoch such that the deal's last updated field is set to the start epoch of the deal
    // and the next tick for it is scheduled at the endepoch.
    rt.set_epoch(process_epoch(start_epoch, deal_id));
    cron_tick(&mut rt);

    // update last updated to some time in the future (breaks state invariants)
    update_last_updated(&mut rt, deal_id, end_epoch + 1000);

    // set current epoch of the deal to the end epoch so it's picked up for "processing" in the next cron tick.
    rt.set_epoch(end_epoch);

    expect_abort(ExitCode::USR_ILLEGAL_STATE, cron_tick_raw(&mut rt));

    check_state_with_expected(
        &rt,
        &[Regex::new("deal \\d+ last updated epoch \\d+ after current \\d+").unwrap()],
    );
}

#[test]
fn crontick_for_a_deal_at_its_start_epoch_results_in_zero_payment_and_no_slashing() {
    let start_epoch = ChainEpoch::from(50);
    let end_epoch = start_epoch + 200 * EPOCHS_IN_DAY;
    let sector_expiry = end_epoch + 100;

    // set start epoch to coincide with processing (0 + 0 % 2880 = 0)
    let start_epoch = 0;
    let mut rt = setup();
    let deal_id = publish_and_activate_deal(
        &mut rt,
        CLIENT_ADDR,
        &MinerAddresses::default(),
        start_epoch,
        end_epoch,
        0,
        sector_expiry,
    );

    // move the current epoch to processing epoch
    let current = process_epoch(start_epoch, deal_id);
    rt.set_epoch(current);
    let (pay, slashed) =
        cron_tick_and_assert_balances(&mut rt, CLIENT_ADDR, PROVIDER_ADDR, current, deal_id);
    assert_eq!(TokenAmount::zero(), pay);
    assert_eq!(TokenAmount::zero(), slashed);

    // deal proposal and state should NOT be deleted
    get_deal_proposal(&mut rt, deal_id);
    get_deal_state(&mut rt, deal_id);
    check_state(&rt);
}

#[test]
fn slash_a_deal_and_make_payment_for_another_deal_in_the_same_epoch() {
    let start_epoch = ChainEpoch::from(50);
    let end_epoch = start_epoch + 200 * EPOCHS_IN_DAY;
    let sector_expiry = end_epoch + 100;

    let mut rt = setup();

    let deal_id1 = publish_and_activate_deal(
        &mut rt,
        CLIENT_ADDR,
        &MinerAddresses::default(),
        start_epoch,
        end_epoch,
        0,
        sector_expiry,
    );
    let d1 = get_deal_proposal(&mut rt, deal_id1);

    let deal_id2 = publish_and_activate_deal(
        &mut rt,
        CLIENT_ADDR,
        &MinerAddresses::default(),
        start_epoch + 1,
        end_epoch + 1,
        0,
        sector_expiry,
    );

    // slash deal1
    let slash_epoch = process_epoch(start_epoch, deal_id2) + ChainEpoch::from(100);
    rt.set_epoch(slash_epoch);
    terminate_deals(&mut rt, PROVIDER_ADDR, &[deal_id1]);

    // cron tick will slash deal1 and make payment for deal2
    rt.expect_send(
        BURNT_FUNDS_ACTOR_ADDR,
        METHOD_SEND,
        RawBytes::default(),
        d1.provider_collateral.clone(),
        RawBytes::default(),
        ExitCode::OK,
    );
    cron_tick(&mut rt);

    assert_deal_deleted(&mut rt, deal_id1, d1);
    let s2 = get_deal_state(&mut rt, deal_id2);
    assert_eq!(slash_epoch, s2.last_updated_epoch);
    check_state(&rt);
}

#[test]
fn cannot_publish_the_same_deal_twice_before_a_cron_tick() {
    let start_epoch = ChainEpoch::from(50);
    let end_epoch = start_epoch + 200 * EPOCHS_IN_DAY;

    // Publish a deal
    let mut rt = setup();
    generate_and_publish_deal(
        &mut rt,
        CLIENT_ADDR,
        &MinerAddresses::default(),
        start_epoch,
        end_epoch,
    );

    // now try to publish it again and it should fail because it will still be in pending state
    let d2 = generate_deal_and_add_funds(
        &mut rt,
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
    rt.expect_validate_caller_type((*CALLER_TYPES_SIGNABLE).to_vec());
    expect_provider_control_address(&mut rt, PROVIDER_ADDR, OWNER_ADDR, WORKER_ADDR);
    expect_query_network_info(&mut rt);
    rt.set_caller(*ACCOUNT_ACTOR_CODE_ID, WORKER_ADDR);

    let auth_param = RawBytes::serialize(AuthenticateMessageParams {
        signature: buf.to_vec(),
        message: buf.to_vec(),
    })
    .unwrap();

    rt.expect_send(
        d2.client,
        AUTHENTICATE_MESSAGE_METHOD,
        auth_param,
        TokenAmount::zero(),
        RawBytes::default(),
        ExitCode::OK,
    );

    expect_abort(
        ExitCode::USR_ILLEGAL_ARGUMENT,
        rt.call::<MarketActor>(
            Method::PublishStorageDeals as u64,
            &RawBytes::serialize(params).unwrap(),
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

    let mut rt = setup();
    let deal_id = generate_and_publish_deal(
        &mut rt,
        CLIENT_ADDR,
        &MinerAddresses::default(),
        start_epoch,
        end_epoch,
    );

    rt.expect_validate_caller_type(vec![Type::Miner]);
    rt.set_caller(*MINER_ACTOR_CODE_ID, PROVIDER_ADDR);
    rt.set_epoch(start_epoch + 1);
    let params = ActivateDealsParams { deal_ids: vec![deal_id], sector_expiry };
    expect_abort(
        ExitCode::USR_ILLEGAL_ARGUMENT,
        rt.call::<MarketActor>(Method::ActivateDeals as u64, &RawBytes::serialize(params).unwrap()),
    );

    rt.verify();
    check_state(&rt);
}

#[test]
fn fail_when_end_epoch_of_deal_greater_than_sector_expiry() {
    let start_epoch = 10;
    let end_epoch = start_epoch + 200 * EPOCHS_IN_DAY;

    let mut rt = setup();
    let deal_id = generate_and_publish_deal(
        &mut rt,
        CLIENT_ADDR,
        &MinerAddresses::default(),
        start_epoch,
        end_epoch,
    );

    rt.expect_validate_caller_type(vec![Type::Miner]);
    rt.set_caller(*MINER_ACTOR_CODE_ID, PROVIDER_ADDR);
    let params = ActivateDealsParams { deal_ids: vec![deal_id], sector_expiry: end_epoch - 1 };
    expect_abort(
        ExitCode::USR_ILLEGAL_ARGUMENT,
        rt.call::<MarketActor>(Method::ActivateDeals as u64, &RawBytes::serialize(params).unwrap()),
    );

    rt.verify();
    check_state(&rt);
}

#[test]
fn fail_to_activate_all_deals_if_one_deal_fails() {
    let start_epoch = 10;
    let end_epoch = start_epoch + 200 * EPOCHS_IN_DAY;
    let sector_expiry = end_epoch + 100;

    let mut rt = setup();
    // activate deal1 so it fails later
    let deal_id1 = generate_and_publish_deal(
        &mut rt,
        CLIENT_ADDR,
        &MinerAddresses::default(),
        start_epoch,
        end_epoch,
    );
    activate_deals(&mut rt, sector_expiry, PROVIDER_ADDR, 0, &[deal_id1]);

    let deal_id2 = generate_and_publish_deal(
        &mut rt,
        CLIENT_ADDR,
        &MinerAddresses::default(),
        start_epoch,
        end_epoch + 1,
    );

    rt.expect_validate_caller_type(vec![Type::Miner]);
    rt.set_caller(*MINER_ACTOR_CODE_ID, PROVIDER_ADDR);
    let params = ActivateDealsParams { deal_ids: vec![deal_id1, deal_id2], sector_expiry };
    expect_abort(
        ExitCode::USR_ILLEGAL_ARGUMENT,
        rt.call::<MarketActor>(Method::ActivateDeals as u64, &RawBytes::serialize(params).unwrap()),
    );
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

    let mut rt = setup();
    rt.actor_code_cids.insert(p1, *MINER_ACTOR_CODE_ID);
    rt.actor_code_cids.insert(c1, *ACCOUNT_ACTOR_CODE_ID);
    let st: State = rt.get_state();

    // assert values are zero
    assert!(st.total_client_locked_collateral.is_zero());
    assert!(st.total_provider_locked_collateral.is_zero());
    assert!(st.total_client_storage_fee.is_zero());

    // Publish deal1, deal2, and deal3 with different client and provider
    let deal_id1 = generate_and_publish_deal(&mut rt, c1, &m1, start_epoch, end_epoch);
    let d1 = get_deal_proposal(&mut rt, deal_id1);

    let deal_id2 = generate_and_publish_deal(&mut rt, c2, &m2, start_epoch, end_epoch);
    let d2 = get_deal_proposal(&mut rt, deal_id2);

    let deal_id3 = generate_and_publish_deal(&mut rt, c3, &m3, start_epoch, end_epoch);
    let d3 = get_deal_proposal(&mut rt, deal_id3);

    let csf = d1.total_storage_fee() + d2.total_storage_fee() + d3.total_storage_fee();
    let plc = &d1.provider_collateral + d2.provider_collateral + &d3.provider_collateral;
    let clc = d1.client_collateral + d2.client_collateral + &d3.client_collateral;

    assert_locked_fund_states(&rt, csf.clone(), plc.clone(), clc.clone());

    // activation doesn't change anything
    let curr = start_epoch - 1;
    rt.set_epoch(curr);
    activate_deals(&mut rt, sector_expiry, p1, curr, &[deal_id1]);
    activate_deals(&mut rt, sector_expiry, p2, curr, &[deal_id2]);

    assert_locked_fund_states(&rt, csf.clone(), plc.clone(), clc.clone());

    // make payment for p1 and p2, p3 times out as it has not been activated
    let curr = process_epoch(start_epoch, deal_id3);
    rt.set_epoch(curr);
    rt.expect_send(
        BURNT_FUNDS_ACTOR_ADDR,
        METHOD_SEND,
        RawBytes::default(),
        d3.provider_collateral.clone(),
        RawBytes::default(),
        ExitCode::OK,
    );
    cron_tick(&mut rt);
    let duration = curr - start_epoch;
    let payment: TokenAmount = 2 * &d1.storage_price_per_epoch * duration;
    let mut csf = (csf - payment) - d3.total_storage_fee();
    let mut plc = plc - d3.provider_collateral;
    let mut clc = clc - d3.client_collateral;
    assert_locked_fund_states(&rt, csf.clone(), plc.clone(), clc.clone());

    // deal1 and deal2 will now be charged at epoch curr + market.DealUpdatesInterval, so nothing changes before that.
    let deal_updates_interval = Policy::default().deal_updates_interval;
    let curr = curr + deal_updates_interval - 1;
    rt.set_epoch(curr);
    cron_tick(&mut rt);
    assert_locked_fund_states(&rt, csf.clone(), plc.clone(), clc.clone());

    // one more round of payment for deal1 and deal2
    let curr = curr + 1;
    rt.set_epoch(curr);
    let duration = deal_updates_interval;
    let payment = 2 * d1.storage_price_per_epoch * duration;
    csf -= payment;
    cron_tick(&mut rt);
    assert_locked_fund_states(&rt, csf.clone(), plc.clone(), clc.clone());

    // slash deal1
    rt.set_epoch(curr + 1);
    terminate_deals(&mut rt, m1.provider, &[deal_id1]);

    // cron tick to slash deal1 and expire deal2
    rt.set_epoch(end_epoch);
    csf = TokenAmount::zero();
    clc = TokenAmount::zero();
    plc = TokenAmount::zero();
    rt.expect_send(
        BURNT_FUNDS_ACTOR_ADDR,
        METHOD_SEND,
        RawBytes::default(),
        d1.provider_collateral,
        RawBytes::default(),
        ExitCode::OK,
    );
    cron_tick(&mut rt);
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
    let mut rt = setup();
    let miner_addresses = MinerAddresses {
        owner: OWNER_ADDR,
        worker: WORKER_ADDR,
        provider: PROVIDER_ADDR,
        control: vec![],
    };

    // test adding provider funds
    let funds = TokenAmount::from_atto(20_000_000);
    add_provider_funds(&mut rt, funds.clone(), &MinerAddresses::default());
    assert_eq!(funds, get_escrow_balance(&rt, &PROVIDER_ADDR).unwrap());

    add_participant_funds(&mut rt, CLIENT_ADDR, funds);
    let mut deal_proposal =
        generate_deal_proposal(CLIENT_ADDR, PROVIDER_ADDR, 1, 200 * EPOCHS_IN_DAY);

    // First attempt at publishing the deal should work
    rt.set_caller(*ACCOUNT_ACTOR_CODE_ID, WORKER_ADDR);
    publish_deals(&mut rt, &miner_addresses, &[deal_proposal.clone()], 1);

    // Second attempt at publishing the same deal should fail
    publish_deals_expect_abort(
        &mut rt,
        &miner_addresses,
        deal_proposal.clone(),
        ExitCode::USR_ILLEGAL_ARGUMENT,
    );

    // Same deal with a different label should work
    deal_proposal.label = Label::String("Cthulhu".to_owned());
    rt.set_caller(*ACCOUNT_ACTOR_CODE_ID, WORKER_ADDR);
    publish_deals(&mut rt, &miner_addresses, &[deal_proposal], 1);
    check_state(&rt);
}

#[test]
fn max_deal_label_size() {
    let mut rt = setup();
    let miner_addresses = MinerAddresses {
        owner: OWNER_ADDR,
        worker: WORKER_ADDR,
        provider: PROVIDER_ADDR,
        control: vec![],
    };

    // Test adding provider funds from both worker and owner address
    let funds = TokenAmount::from_atto(20_000_000);
    add_provider_funds(&mut rt, funds.clone(), &MinerAddresses::default());
    assert_eq!(funds, get_escrow_balance(&rt, &PROVIDER_ADDR).unwrap());

    add_participant_funds(&mut rt, CLIENT_ADDR, funds);
    let mut deal_proposal =
        generate_deal_proposal(CLIENT_ADDR, PROVIDER_ADDR, 1, 200 * EPOCHS_IN_DAY);

    // DealLabel at max size should work.
    deal_proposal.label = Label::String("s".repeat(DEAL_MAX_LABEL_SIZE));
    rt.set_caller(*ACCOUNT_ACTOR_CODE_ID, WORKER_ADDR);
    publish_deals(&mut rt, &miner_addresses, &[deal_proposal.clone()], 1);

    // over max should fail
    deal_proposal.label = Label::String("s".repeat(DEAL_MAX_LABEL_SIZE + 1));
    publish_deals_expect_abort(
        &mut rt,
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
    let mut rt = setup();

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
    add_participant_funds(&mut rt, CLIENT_ADDR, deal2.client_balance_requirement());

    // Provider has enough for both
    let provider_funds =
        deal1.provider_balance_requirement().add(deal2.provider_balance_requirement());
    rt.set_caller(*ACCOUNT_ACTOR_CODE_ID, OWNER_ADDR);
    rt.set_value(provider_funds);
    rt.expect_validate_caller_type((*CALLER_TYPES_SIGNABLE).to_vec());
    expect_provider_control_address(&mut rt, PROVIDER_ADDR, OWNER_ADDR, WORKER_ADDR);

    assert_eq!(
        RawBytes::default(),
        rt.call::<MarketActor>(
            Method::AddBalance as u64,
            &RawBytes::serialize(PROVIDER_ADDR).unwrap(),
        )
        .unwrap()
    );

    rt.verify();

    assert_eq!(deal2.client_balance_requirement(), get_escrow_balance(&rt, &CLIENT_ADDR).unwrap());
    assert_eq!(
        deal1.provider_balance_requirement().add(deal2.provider_balance_requirement()),
        get_escrow_balance(&rt, &PROVIDER_ADDR).unwrap()
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

    rt.expect_validate_caller_type((*CALLER_TYPES_SIGNABLE).to_vec());
    expect_provider_control_address(&mut rt, PROVIDER_ADDR, OWNER_ADDR, WORKER_ADDR);
    expect_query_network_info(&mut rt);

    let authenticate_param1 = RawBytes::serialize(AuthenticateMessageParams {
        signature: buf1.to_vec(),
        message: buf1.to_vec(),
    })
    .unwrap();
    let authenticate_param2 = RawBytes::serialize(AuthenticateMessageParams {
        signature: buf2.to_vec(),
        message: buf2.to_vec(),
    })
    .unwrap();

    rt.expect_send(
        deal1.client,
        AUTHENTICATE_MESSAGE_METHOD as u64,
        authenticate_param1,
        TokenAmount::zero(),
        RawBytes::default(),
        ExitCode::OK,
    );
    rt.expect_send(
        deal2.client,
        AUTHENTICATE_MESSAGE_METHOD as u64,
        authenticate_param2,
        TokenAmount::zero(),
        RawBytes::default(),
        ExitCode::OK,
    );

    rt.set_caller(*ACCOUNT_ACTOR_CODE_ID, WORKER_ADDR);

    let ret: PublishStorageDealsReturn = rt
        .call::<MarketActor>(
            Method::PublishStorageDeals as u64,
            &RawBytes::serialize(params).unwrap(),
        )
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
    let mut rt = setup();

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
        &mut rt,
        CLIENT_ADDR,
        deal1.client_balance_requirement().add(deal2.client_balance_requirement()),
    );

    // Provider has enough for only the second deal
    rt.set_caller(*ACCOUNT_ACTOR_CODE_ID, OWNER_ADDR);
    rt.set_value(deal2.provider_balance_requirement().clone());
    rt.expect_validate_caller_type((*CALLER_TYPES_SIGNABLE).to_vec());
    expect_provider_control_address(&mut rt, PROVIDER_ADDR, OWNER_ADDR, WORKER_ADDR);

    assert_eq!(
        RawBytes::default(),
        rt.call::<MarketActor>(
            Method::AddBalance as u64,
            &RawBytes::serialize(PROVIDER_ADDR).unwrap(),
        )
        .unwrap()
    );

    rt.verify();

    assert_eq!(
        deal1.client_balance_requirement().add(deal2.client_balance_requirement()),
        get_escrow_balance(&rt, &CLIENT_ADDR).unwrap()
    );
    assert_eq!(
        deal2.provider_balance_requirement().clone(),
        get_escrow_balance(&rt, &PROVIDER_ADDR).unwrap()
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

    rt.expect_validate_caller_type((*CALLER_TYPES_SIGNABLE).to_vec());
    expect_provider_control_address(&mut rt, PROVIDER_ADDR, OWNER_ADDR, WORKER_ADDR);
    expect_query_network_info(&mut rt);

    let authenticate_param1 = RawBytes::serialize(AuthenticateMessageParams {
        signature: buf1.to_vec(),
        message: buf1.to_vec(),
    })
    .unwrap();
    let authenticate_param2 = RawBytes::serialize(AuthenticateMessageParams {
        signature: buf2.to_vec(),
        message: buf2.to_vec(),
    })
    .unwrap();

    rt.expect_send(
        deal1.client,
        AUTHENTICATE_MESSAGE_METHOD as u64,
        authenticate_param1,
        TokenAmount::zero(),
        RawBytes::default(),
        ExitCode::OK,
    );
    rt.expect_send(
        deal2.client,
        AUTHENTICATE_MESSAGE_METHOD as u64,
        authenticate_param2,
        TokenAmount::zero(),
        RawBytes::default(),
        ExitCode::OK,
    );

    rt.set_caller(*ACCOUNT_ACTOR_CODE_ID, WORKER_ADDR);

    let ret: PublishStorageDealsReturn = rt
        .call::<MarketActor>(
            Method::PublishStorageDeals as u64,
            &RawBytes::serialize(params).unwrap(),
        )
        .unwrap()
        .deserialize()
        .unwrap();

    assert!(ret.valid_deals.get(1));
    assert!(!ret.valid_deals.get(0));

    rt.verify();

    check_state(&rt);
}
