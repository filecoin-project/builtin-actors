// Copyright 2019-2022 ChainSafe Systems
// SPDX-License-Identifier: Apache-2.0, MIT

use std::convert::TryInto;

use fil_actor_market::balance_table::BALANCE_TABLE_BITWIDTH;
use fil_actor_market::{
    ext, ActivateDealsParams, Actor as MarketActor, ClientDealProposal, DealMetaArray, Label,
    Method, OnMinerSectorsTerminateParams, PublishStorageDealsParams, PublishStorageDealsReturn,
    State, WithdrawBalanceParams, PROPOSALS_AMT_BITWIDTH, STATES_AMT_BITWIDTH,
};
use fil_actor_verifreg::UseBytesParams;
use fil_actors_runtime::cbor::deserialize;
use fil_actors_runtime::network::EPOCHS_IN_DAY;
use fil_actors_runtime::runtime::Policy;
use fil_actors_runtime::test_utils::*;
use fil_actors_runtime::{
    make_empty_map, ActorError, SetMultimap, BURNT_FUNDS_ACTOR_ADDR, SYSTEM_ACTOR_ADDR,
    VERIFIED_REGISTRY_ACTOR_ADDR,
};
use fvm_ipld_amt::Amt;
use fvm_ipld_encoding::{to_vec, RawBytes};
use fvm_shared::address::Address;
use fvm_shared::bigint::bigint_ser::BigIntDe;
use fvm_shared::bigint::BigInt;
use fvm_shared::clock::{ChainEpoch, EPOCH_UNDEFINED};
use fvm_shared::crypto::signature::Signature;
use fvm_shared::deal::DealID;
use fvm_shared::econ::TokenAmount;
use fvm_shared::error::ExitCode;
use fvm_shared::piece::PaddedPieceSize;
use fvm_shared::sector::StoragePower;
use fvm_shared::{HAMT_BIT_WIDTH, METHOD_CONSTRUCTOR, METHOD_SEND};

use num_traits::FromPrimitive;

mod harness;
use harness::*;

// TODO add array stuff
#[test]
fn simple_construction() {
    let mut rt = MockRuntime {
        receiver: Address::new_id(100),
        caller: *SYSTEM_ACTOR_ADDR,
        caller_type: *INIT_ACTOR_CODE_ID,
        ..Default::default()
    };

    rt.expect_validate_caller_addr(vec![*SYSTEM_ACTOR_ADDR]);

    assert_eq!(
        RawBytes::default(),
        rt.call::<MarketActor>(METHOD_CONSTRUCTOR, &RawBytes::default(),).unwrap()
    );

    rt.verify();

    let store = &rt.store;

    let empty_balance_table =
        make_empty_map::<_, BigIntDe>(store, BALANCE_TABLE_BITWIDTH).flush().unwrap();
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
            rt.set_value(TokenAmount::from(tc.delta));
            rt.expect_validate_caller_type((*CALLER_TYPES_SIGNABLE).clone());
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
                TokenAmount::from(tc.total)
            );
            check_state(&rt);
        }
    }
}

#[test]
fn fails_if_withdraw_from_non_provider_funds_is_not_initiated_by_the_recipient() {
    let mut rt = setup();

    add_participant_funds(&mut rt, CLIENT_ADDR, TokenAmount::from(20u8));

    assert_eq!(TokenAmount::from(20u8), get_escrow_balance(&rt, &CLIENT_ADDR).unwrap());

    rt.expect_validate_caller_addr(vec![CLIENT_ADDR]);

    let params =
        WithdrawBalanceParams { provider_or_client: CLIENT_ADDR, amount: TokenAmount::from(1u8) };

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
    assert_eq!(TokenAmount::from(20u8), get_escrow_balance(&rt, &CLIENT_ADDR).unwrap());

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

    let withdraw_amount = TokenAmount::from(1u8);
    let withdrawable_amount = TokenAmount::from(0u8);
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
    let withdraw_amount = TokenAmount::from(30u8);
    let withdrawable_amount = TokenAmount::from(25u8);

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
    let withdraw_amount = TokenAmount::from(1);
    let actual_withdrawn = TokenAmount::from(0);
    withdraw_provider_balance(
        &mut rt,
        withdraw_amount,
        actual_withdrawn,
        PROVIDER_ADDR,
        OWNER_ADDR,
        WORKER_ADDR,
    );

    // add some more funds to the provider & ensure withdrawal is limited by the locked funds
    add_provider_funds(&mut rt, TokenAmount::from(25), &MinerAddresses::default());
    let withdraw_amount = TokenAmount::from(30);
    let actual_withdrawn = TokenAmount::from(25);

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

    rt.set_value(TokenAmount::from(10));
    rt.expect_validate_caller_type((*CALLER_TYPES_SIGNABLE).clone());

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
            rt.set_value(TokenAmount::from(tc.delta));
            rt.expect_validate_caller_type((*CALLER_TYPES_SIGNABLE).clone());

            assert_eq!(
                RawBytes::default(),
                rt.call::<MarketActor>(
                    Method::AddBalance as u64,
                    &RawBytes::serialize(caller_addr).unwrap(),
                )
                .unwrap()
            );

            rt.verify();

            assert_eq!(get_escrow_balance(&rt, caller_addr).unwrap(), TokenAmount::from(tc.total));
            check_state(&rt);
        }
    }
}

#[test]
fn withdraws_from_provider_escrow_funds_and_sends_to_owner() {
    let mut rt = setup();

    let amount = TokenAmount::from(20);
    add_provider_funds(&mut rt, amount.clone(), &MinerAddresses::default());

    assert_eq!(amount, get_escrow_balance(&rt, &PROVIDER_ADDR).unwrap());

    // worker calls WithdrawBalance, balance is transferred to owner
    let withdraw_amount = TokenAmount::from(1);
    withdraw_provider_balance(
        &mut rt,
        withdraw_amount.clone(),
        withdraw_amount,
        PROVIDER_ADDR,
        OWNER_ADDR,
        WORKER_ADDR,
    );

    assert_eq!(TokenAmount::from(19), get_escrow_balance(&rt, &PROVIDER_ADDR).unwrap());
    check_state(&rt);
}

#[test]
fn withdraws_from_non_provider_escrow_funds() {
    let mut rt = setup();

    let amount = TokenAmount::from(20);
    add_participant_funds(&mut rt, CLIENT_ADDR, amount.clone());

    assert_eq!(get_escrow_balance(&rt, &CLIENT_ADDR).unwrap(), amount);

    let withdraw_amount = TokenAmount::from(1);
    withdraw_client_balance(&mut rt, withdraw_amount.clone(), withdraw_amount, CLIENT_ADDR);

    assert_eq!(get_escrow_balance(&rt, &CLIENT_ADDR).unwrap(), TokenAmount::from(19));
    check_state(&rt);
}

#[test]
fn client_withdrawing_more_than_escrow_balance_limits_to_available_funds() {
    let mut rt = setup();

    let amount = TokenAmount::from(20);
    add_participant_funds(&mut rt, CLIENT_ADDR, amount.clone());

    // withdraw amount greater than escrow balance
    let withdraw_amount = TokenAmount::from(25);
    withdraw_client_balance(&mut rt, withdraw_amount, amount, CLIENT_ADDR);

    assert_eq!(get_escrow_balance(&rt, &CLIENT_ADDR).unwrap(), TokenAmount::from(0));
}

#[test]
fn worker_withdrawing_more_than_escrow_balance_limits_to_available_funds() {
    let mut rt = setup();

    let amount = TokenAmount::from(20);
    add_provider_funds(&mut rt, amount.clone(), &MinerAddresses::default());

    assert_eq!(get_escrow_balance(&rt, &PROVIDER_ADDR).unwrap(), amount);

    let withdraw_amount = TokenAmount::from(25);
    withdraw_provider_balance(
        &mut rt,
        withdraw_amount,
        amount,
        PROVIDER_ADDR,
        OWNER_ADDR,
        WORKER_ADDR,
    );

    assert_eq!(get_escrow_balance(&rt, &PROVIDER_ADDR).unwrap(), TokenAmount::from(0));
    check_state(&rt);
}

#[test]
fn fail_when_balance_is_zero() {
    let mut rt = setup();

    rt.set_caller(*ACCOUNT_ACTOR_CODE_ID, OWNER_ADDR);
    rt.set_received(BigInt::from(0_i32));

    expect_abort(
        ExitCode::USR_ILLEGAL_ARGUMENT,
        rt.call::<MarketActor>(
            Method::AddBalance as u64,
            &RawBytes::serialize(&PROVIDER_ADDR).unwrap(),
        ),
    );

    rt.verify();
}

#[test]
fn fails_with_a_negative_withdraw_amount() {
    let mut rt = setup();

    let params = WithdrawBalanceParams {
        provider_or_client: PROVIDER_ADDR,
        amount: TokenAmount::from(-1_i32),
    };

    expect_abort(
        ExitCode::USR_ILLEGAL_ARGUMENT,
        rt.call::<MarketActor>(
            Method::WithdrawBalance as u64,
            &RawBytes::serialize(&params).unwrap(),
        ),
    );

    rt.verify();
}

#[test]
fn fails_if_withdraw_from_provider_funds_is_not_initiated_by_the_owner_or_worker() {
    let mut rt = setup();

    let amount = TokenAmount::from(20u8);
    add_provider_funds(&mut rt, amount.clone(), &MinerAddresses::default());

    assert_eq!(get_escrow_balance(&rt, &PROVIDER_ADDR).unwrap(), amount);

    // only signing parties can add balance for client AND provider.
    rt.expect_validate_caller_addr(vec![OWNER_ADDR, WORKER_ADDR]);
    let params =
        WithdrawBalanceParams { provider_or_client: PROVIDER_ADDR, amount: TokenAmount::from(1u8) };

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

    // Publish from miner worker.
    let deal1 = generate_deal_and_add_funds(
        &mut rt,
        CLIENT_ADDR,
        &MinerAddresses::default(),
        start_epoch,
        end_epoch,
    );
    rt.set_caller(*ACCOUNT_ACTOR_CODE_ID, WORKER_ADDR);
    publish_deals(&mut rt, &MinerAddresses::default(), &[deal1]);

    // Publish from miner control address.
    let deal2 = generate_deal_and_add_funds(
        &mut rt,
        CLIENT_ADDR,
        &MinerAddresses::default(),
        start_epoch + 1,
        end_epoch + 1,
    );
    rt.set_caller(*ACCOUNT_ACTOR_CODE_ID, CONTROL_ADDR);
    publish_deals(&mut rt, &MinerAddresses::default(), &[deal2]);
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

    // add funds for cient using it's BLS address -> will be resolved and persisted
    add_participant_funds(&mut rt, client_bls, deal.client_balance_requirement());
    assert_eq!(
        deal.client_balance_requirement(),
        get_escrow_balance(&rt, &client_resolved).unwrap()
    );

    // add funds for provider using it's BLS address -> will be resolved and persisted
    rt.value_received = deal.provider_collateral.clone();
    rt.set_caller(*ACCOUNT_ACTOR_CODE_ID, OWNER_ADDR);
    rt.expect_validate_caller_type((*CALLER_TYPES_SIGNABLE).clone());
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
    rt.expect_validate_caller_type((*CALLER_TYPES_SIGNABLE).clone());

    expect_provider_control_address(&mut rt, provider_resolved, OWNER_ADDR, WORKER_ADDR);
    expect_query_network_info(&mut rt);

    //  create a client proposal with a valid signature
    let mut params = PublishStorageDealsParams { deals: vec![] };
    let buf = RawBytes::serialize(&deal).expect("failed to marshal deal proposal");
    let sig = Signature::new_bls("does not matter".as_bytes().to_vec());
    let client_proposal =
        ClientDealProposal { client_signature: sig.clone(), proposal: deal.clone() };
    params.deals.push(client_proposal);
    // expect a call to verify the above signature
    rt.expect_verify_signature(ExpectedVerifySig {
        sig,
        signer: deal.client,
        plaintext: buf.to_vec(),
        result: Ok(()),
    });

    // request is sent to the VerigReg actor using the resolved address
    let param = RawBytes::serialize(UseBytesParams {
        address: client_resolved,
        deal_size: BigInt::from(deal.piece_size.0),
    })
    .unwrap();

    rt.expect_send(
        *VERIFIED_REGISTRY_ACTOR_ADDR,
        ext::verifreg::USE_BYTES_METHOD as u64,
        param,
        TokenAmount::from(0u8),
        RawBytes::default(),
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

// Converted from https://github.com/filecoin-project/specs-actors/blob/d56b240af24517443ce1f8abfbdab7cb22d331f1/actors/builtin/market/market_test.go#L1274
#[test]
fn terminate_multiple_deals_from_multiple_providers() {
    let start_epoch = 10;
    let end_epoch = start_epoch + 200 * EPOCHS_IN_DAY;
    let sector_expiry = end_epoch + 100;
    let current_epoch = 5;

    let provider2 = Address::new_id(501);

    let mut rt = setup();
    rt.set_epoch(current_epoch);

    let [deal1, deal2, deal3]: [DealID; 3] = (end_epoch..end_epoch + 3)
        .map(|epoch| {
            generate_and_publish_deal(
                &mut rt,
                CLIENT_ADDR,
                &MinerAddresses::default(),
                start_epoch,
                epoch,
            )
        })
        .collect::<Vec<DealID>>()
        .try_into()
        .unwrap();
    activate_deals(&mut rt, sector_expiry, PROVIDER_ADDR, current_epoch, &[deal1, deal2, deal3]);

    let addrs = MinerAddresses { provider: provider2, ..MinerAddresses::default() };
    let deal4 = generate_and_publish_deal(&mut rt, CLIENT_ADDR, &addrs, start_epoch, end_epoch);
    let deal5 = generate_and_publish_deal(&mut rt, CLIENT_ADDR, &addrs, start_epoch, end_epoch + 1);
    activate_deals(&mut rt, sector_expiry, provider2, current_epoch, &[deal4, deal5]);

    terminate_deals(&mut rt, PROVIDER_ADDR, &[deal1]);
    assert_deals_terminated(&mut rt, current_epoch, &[deal1]);
    assert_deals_not_terminated(&mut rt, &[deal2, deal3, deal4, deal5]);

    terminate_deals(&mut rt, provider2, &[deal5]);
    assert_deals_terminated(&mut rt, current_epoch, &[deal5]);
    assert_deals_not_terminated(&mut rt, &[deal2, deal3, deal4]);

    terminate_deals(&mut rt, PROVIDER_ADDR, &[deal2, deal3]);
    assert_deals_terminated(&mut rt, current_epoch, &[deal2, deal3]);
    assert_deals_not_terminated(&mut rt, &[deal4]);

    terminate_deals(&mut rt, provider2, &[deal4]);
    assert_deals_terminated(&mut rt, current_epoch, &[deal4]);
}

// Converted from: https://github.com/filecoin-project/specs-actors/blob/d56b240af24517443ce1f8abfbdab7cb22d331f1/actors/builtin/market/market_test.go#L1312
#[test]
fn ignore_deal_proposal_that_does_not_exist() {
    let start_epoch = 10;
    let end_epoch = start_epoch + 200 * EPOCHS_IN_DAY;
    let sector_expiry = end_epoch + 100;
    let current_epoch = 5;

    let mut rt = setup();
    rt.set_epoch(current_epoch);

    let deal1 = generate_and_publish_deal(
        &mut rt,
        CLIENT_ADDR,
        &MinerAddresses::default(),
        start_epoch,
        end_epoch,
    );
    activate_deals(&mut rt, sector_expiry, PROVIDER_ADDR, current_epoch, &[deal1]);

    terminate_deals(&mut rt, PROVIDER_ADDR, &[deal1, 42]);

    let s = get_deal_state(&mut rt, deal1);
    assert_eq!(s.slash_epoch, current_epoch);
}

// Converted from: https://github.com/filecoin-project/specs-actors/blob/d56b240af24517443ce1f8abfbdab7cb22d331f1/actors/builtin/market/market_test.go#L1326
#[test]
fn terminate_valid_deals_along_with_just_expired_deal() {
    let start_epoch = 10;
    let end_epoch = start_epoch + 200 * EPOCHS_IN_DAY;
    let sector_expiry = end_epoch + 100;
    let current_epoch = 5;

    let mut rt = setup();
    rt.set_epoch(current_epoch);

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
        end_epoch - 1,
    );
    activate_deals(&mut rt, sector_expiry, PROVIDER_ADDR, current_epoch, &[deal1, deal2, deal3]);

    let new_epoch = end_epoch - 1;
    rt.set_epoch(new_epoch);

    terminate_deals(&mut rt, PROVIDER_ADDR, &[deal1, deal2, deal3]);
    assert_deals_terminated(&mut rt, new_epoch, &[deal1, deal2]);
    assert_deals_not_terminated(&mut rt, &[deal3]);
}
// Converted from: https://github.com/filecoin-project/specs-actors/blob/d56b240af24517443ce1f8abfbdab7cb22d331f1/actors/builtin/market/market_test.go#L1346
#[test]
fn terminate_valid_deals_along_with_expired_and_cleaned_up_deal() {
    let deal_updates_interval = Policy::default().deal_updates_interval;
    let start_epoch = 10;
    let end_epoch = start_epoch + 200 * EPOCHS_IN_DAY;
    let sector_expiry = end_epoch + 100;
    let current_epoch = 5;

    let mut rt = setup();
    rt.set_epoch(current_epoch);

    let deal1 = generate_deal_and_add_funds(
        &mut rt,
        CLIENT_ADDR,
        &MinerAddresses::default(),
        start_epoch,
        end_epoch,
    );
    let deal2 = generate_deal_and_add_funds(
        &mut rt,
        CLIENT_ADDR,
        &MinerAddresses::default(),
        start_epoch,
        end_epoch - deal_updates_interval,
    );

    rt.set_caller(*ACCOUNT_ACTOR_CODE_ID, WORKER_ADDR);
    let deal_ids = publish_deals(&mut rt, &MinerAddresses::default(), &[deal1, deal2.clone()]);
    activate_deals(&mut rt, sector_expiry, PROVIDER_ADDR, current_epoch, &deal_ids);

    let new_epoch = end_epoch - 1;
    rt.set_epoch(new_epoch);
    cron_tick(&mut rt);

    terminate_deals(&mut rt, PROVIDER_ADDR, &deal_ids);
    assert_deals_terminated(&mut rt, new_epoch, &deal_ids[0..0]);
    assert_deal_deleted(&mut rt, deal_ids[1], deal2);
}

// Converted from: https://github.com/filecoin-project/specs-actors/blob/d56b240af24517443ce1f8abfbdab7cb22d331f1/actors/builtin/market/market_test.go#L1369
#[test]
fn terminating_a_deal_the_second_time_does_not_change_its_slash_epoch() {
    let start_epoch = 10;
    let end_epoch = start_epoch + 200 * EPOCHS_IN_DAY;
    let sector_expiry = end_epoch + 100;
    let current_epoch = 5;

    let mut rt = setup();
    rt.set_epoch(current_epoch);

    let deal1 = generate_and_publish_deal(
        &mut rt,
        CLIENT_ADDR,
        &MinerAddresses::default(),
        start_epoch,
        end_epoch,
    );
    activate_deals(&mut rt, sector_expiry, PROVIDER_ADDR, current_epoch, &[deal1]);

    // terminating the deal so slash epoch is the current epoch
    terminate_deals(&mut rt, PROVIDER_ADDR, &[deal1]);

    // set a new epoch and terminate again -> however slash epoch will still be the old epoch.
    rt.set_epoch(current_epoch + 1);
    terminate_deals(&mut rt, PROVIDER_ADDR, &[deal1]);
    let s = get_deal_state(&mut rt, deal1);
    assert_eq!(s.slash_epoch, current_epoch);
}

// Converted from: https://github.com/filecoin-project/specs-actors/blob/d56b240af24517443ce1f8abfbdab7cb22d331f1/actors/builtin/market/market_test.go#L1387
#[test]
fn terminating_new_deals_and_an_already_terminated_deal_only_terminates_the_new_deals() {
    let start_epoch = 10;
    let end_epoch = start_epoch + 200 * EPOCHS_IN_DAY;
    let sector_expiry = end_epoch + 100;
    let current_epoch = 5;

    let mut rt = setup();
    rt.set_epoch(current_epoch);

    // provider1 publishes deal1 and 2 and deal3 -> deal3 has the lowest endepoch
    let deals: Vec<DealID> = [end_epoch, end_epoch + 1, end_epoch - 1]
        .iter()
        .map(|&epoch| {
            generate_and_publish_deal(
                &mut rt,
                CLIENT_ADDR,
                &MinerAddresses::default(),
                start_epoch,
                epoch,
            )
        })
        .collect();
    let [deal1, deal2, deal3]: [DealID; 3] = deals.as_slice().try_into().unwrap();
    activate_deals(&mut rt, sector_expiry, PROVIDER_ADDR, current_epoch, &deals);

    // terminating the deal so slash epoch is the current epoch
    terminate_deals(&mut rt, PROVIDER_ADDR, &[deal1]);

    // set a new epoch and terminate again -> however slash epoch will still be the old epoch.
    let new_epoch = current_epoch + 1;
    rt.set_epoch(new_epoch);
    terminate_deals(&mut rt, PROVIDER_ADDR, &deals);

    let s1 = get_deal_state(&mut rt, deal1);
    assert_eq!(s1.slash_epoch, current_epoch);

    let s2 = get_deal_state(&mut rt, deal2);
    assert_eq!(s2.slash_epoch, new_epoch);

    let s3 = get_deal_state(&mut rt, deal3);
    assert_eq!(s3.slash_epoch, new_epoch);
}

// Converted from: https://github.com/filecoin-project/specs-actors/blob/d56b240af24517443ce1f8abfbdab7cb22d331f1/actors/builtin/market/market_test.go#L1415
#[test]
fn do_not_terminate_deal_if_end_epoch_is_equal_to_or_less_than_current_epoch() {
    let start_epoch = 10;
    let end_epoch = start_epoch + 200 * EPOCHS_IN_DAY;
    let sector_expiry = end_epoch + 100;
    let current_epoch = 5;

    let mut rt = setup();
    rt.set_epoch(current_epoch);

    // deal1 has endepoch equal to current epoch when terminate is called
    let deal1 = generate_and_publish_deal(
        &mut rt,
        CLIENT_ADDR,
        &MinerAddresses::default(),
        start_epoch,
        end_epoch,
    );
    activate_deals(&mut rt, sector_expiry, PROVIDER_ADDR, current_epoch, &[deal1]);
    rt.set_epoch(end_epoch);
    terminate_deals(&mut rt, PROVIDER_ADDR, &[deal1]);
    assert_deals_not_terminated(&mut rt, &[deal1]);

    // deal2 has end epoch less than current epoch when terminate is called
    rt.set_epoch(current_epoch);
    let deal2 = generate_and_publish_deal(
        &mut rt,
        CLIENT_ADDR,
        &MinerAddresses::default(),
        start_epoch + 1,
        end_epoch,
    );
    activate_deals(&mut rt, sector_expiry, PROVIDER_ADDR, current_epoch, &[deal2]);
    rt.set_epoch(end_epoch + 1);
    terminate_deals(&mut rt, PROVIDER_ADDR, &[deal2]);
    assert_deals_not_terminated(&mut rt, &[deal2]);
}

// Converted from: https://github.com/filecoin-project/specs-actors/blob/master/actors/builtin/market/market_test.go#L1436
#[test]
fn fail_when_caller_is_not_a_storage_miner_actor() {
    let mut rt = setup();
    rt.expect_validate_caller_type(vec![*MINER_ACTOR_CODE_ID]);
    rt.set_caller(*ACCOUNT_ACTOR_CODE_ID, PROVIDER_ADDR);
    let params = OnMinerSectorsTerminateParams { epoch: rt.epoch, deal_ids: vec![] };

    // XXX: Which exit code is correct: SYS_FORBIDDEN(8) or USR_FORBIDDEN(18)?
    assert_eq!(
        ExitCode::USR_FORBIDDEN,
        rt.call::<MarketActor>(
            Method::OnMinerSectorsTerminate as u64,
            &RawBytes::serialize(params).unwrap(),
        )
        .unwrap_err()
        .exit_code()
    );
}

// Converted from: https://github.com/filecoin-project/specs-actors/blob/master/actors/builtin/market/market_test.go#L1448
#[test]
fn fail_when_caller_is_not_the_provider_of_the_deal() {
    let start_epoch = 10;
    let end_epoch = start_epoch + 200 * EPOCHS_IN_DAY;
    let sector_expiry = end_epoch + 100;
    let current_epoch = 5;

    let provider2 = Address::new_id(501);

    let mut rt = setup();
    rt.set_epoch(current_epoch);

    let deal = generate_and_publish_deal(
        &mut rt,
        CLIENT_ADDR,
        &MinerAddresses::default(),
        start_epoch,
        end_epoch,
    );
    activate_deals(&mut rt, sector_expiry, PROVIDER_ADDR, current_epoch, &[deal]);

    // XXX: Difference between go messages: 't0501' has turned into 'f0501'.
    let ret = terminate_deals_raw(&mut rt, provider2, &[deal]);
    expect_abort_contains_message(
        ExitCode::USR_ILLEGAL_STATE,
        "caller f0501 is not the provider f0102 of deal 0",
        ret,
    );
}

// Converted from: https://github.com/filecoin-project/specs-actors/blob/master/actors/builtin/market/market_test.go#L1468
#[test]
fn fail_when_deal_has_been_published_but_not_activated() {
    let start_epoch = 10;
    let end_epoch = start_epoch + 200 * EPOCHS_IN_DAY;
    let current_epoch = 5;

    let mut rt = setup();
    rt.set_epoch(current_epoch);

    let deal = generate_and_publish_deal(
        &mut rt,
        CLIENT_ADDR,
        &MinerAddresses::default(),
        start_epoch,
        end_epoch,
    );

    let ret = terminate_deals_raw(&mut rt, PROVIDER_ADDR, &[deal]);
    expect_abort_contains_message(ExitCode::USR_ILLEGAL_ARGUMENT, "no state for deal", ret);
    rt.verify();
}

// Converted from: https://github.com/filecoin-project/specs-actors/blob/master/actors/builtin/market/market_test.go#L1485
#[test]
fn termination_of_all_deals_should_fail_when_one_deal_fails() {
    let start_epoch = 10;
    let end_epoch = start_epoch + 200 * EPOCHS_IN_DAY;
    let sector_expiry = end_epoch + 100;
    let current_epoch = 5;

    let mut rt = setup();
    rt.set_epoch(current_epoch);

    // deal1 would terminate but deal2 will fail because deal2 has not been activated
    let deal1 = generate_and_publish_deal(
        &mut rt,
        CLIENT_ADDR,
        &MinerAddresses::default(),
        start_epoch,
        end_epoch,
    );
    activate_deals(&mut rt, sector_expiry, PROVIDER_ADDR, current_epoch, &[deal1]);
    let deal2 = generate_and_publish_deal(
        &mut rt,
        CLIENT_ADDR,
        &MinerAddresses::default(),
        start_epoch,
        end_epoch + 1,
    );

    let ret = terminate_deals_raw(&mut rt, PROVIDER_ADDR, &[deal1, deal2]);
    expect_abort_contains_message(ExitCode::USR_ILLEGAL_ARGUMENT, "no state for deal", ret);
    rt.verify();

    // verify deal1 has not been terminated
    assert_deals_not_terminated(&mut rt, &[deal1]);
}

#[test]
fn publish_a_deal_with_enough_collateral_when_circulating_supply_is_superior_to_zero() {
    let policy = Policy::default();

    let start_epoch = 1000;
    let end_epoch = start_epoch + 200 * EPOCHS_IN_DAY;
    let publish_epoch = ChainEpoch::from(1);

    let mut rt = setup();

    let client_collateral = TokenAmount::from(10u8); // min is zero so this is placeholder

    // given power and circ supply cancel this should be 1*dealqapower / 100
    let deal_size = PaddedPieceSize(2048u64); // generateDealProposal's deal size
    let provider_collateral = TokenAmount::from(
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
    rt.set_circulating_supply(qa_power); // convenient for these two numbers to cancel out

    // publish the deal successfully
    rt.set_epoch(publish_epoch);
    rt.set_caller(*ACCOUNT_ACTOR_CODE_ID, WORKER_ADDR);
    publish_deals(&mut rt, &MinerAddresses::default(), &[deal]);
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
    publish_deals(&mut rt, &MinerAddresses::default(), &[deal4.clone(), deal5.clone()]);

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
    publish_deals(&mut rt, &addrs, &[deal6.clone(), deal7.clone()]);

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

    check_state(&rt);
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

    check_state(&rt);
}

#[cfg(test)]
mod test_activate_deal_failures {
    use super::*;

    #[test]
    fn fail_when_caller_is_not_the_provider_of_the_deal() {
        let start_epoch = 10;
        let end_epoch = start_epoch + 200 * EPOCHS_IN_DAY;
        let sector_expiry = end_epoch + 100;

        let mut rt = setup();
        let provider2_addr = Address::new_id(201);
        let addrs = MinerAddresses { provider: provider2_addr, ..MinerAddresses::default() };
        let deal_id =
            generate_and_publish_deal(&mut rt, CLIENT_ADDR, &addrs, start_epoch, end_epoch);

        let params = ActivateDealsParams { deal_ids: vec![deal_id], sector_expiry };

        rt.expect_validate_caller_type(vec![*MINER_ACTOR_CODE_ID]);
        rt.set_caller(*MINER_ACTOR_CODE_ID, PROVIDER_ADDR);
        expect_abort(
            ExitCode::USR_FORBIDDEN,
            rt.call::<MarketActor>(
                Method::ActivateDeals as u64,
                &RawBytes::serialize(params).unwrap(),
            ),
        );

        rt.verify();
        check_state(&rt);
    }

    #[test]
    fn fail_when_caller_is_not_a_storage_miner_actor() {
        let mut rt = setup();
        rt.expect_validate_caller_type(vec![*MINER_ACTOR_CODE_ID]);
        rt.set_caller(*ACCOUNT_ACTOR_CODE_ID, PROVIDER_ADDR);

        let params = ActivateDealsParams { deal_ids: vec![], sector_expiry: 0 };
        expect_abort(
            ExitCode::USR_FORBIDDEN,
            rt.call::<MarketActor>(
                Method::ActivateDeals as u64,
                &RawBytes::serialize(params).unwrap(),
            ),
        );

        rt.verify();
        check_state(&rt);
    }

    #[test]
    fn fail_when_deal_has_not_been_published_before() {
        let mut rt = setup();
        let params = ActivateDealsParams { deal_ids: vec![DealID::from(42u32)], sector_expiry: 0 };

        rt.expect_validate_caller_type(vec![*MINER_ACTOR_CODE_ID]);
        rt.set_caller(*MINER_ACTOR_CODE_ID, PROVIDER_ADDR);
        expect_abort(
            ExitCode::USR_NOT_FOUND,
            rt.call::<MarketActor>(
                Method::ActivateDeals as u64,
                &RawBytes::serialize(params).unwrap(),
            ),
        );

        rt.verify();
        check_state(&rt);
    }

    #[test]
    fn fail_when_deal_has_already_been_activated() {
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
        activate_deals(&mut rt, sector_expiry, PROVIDER_ADDR, 0, &[deal_id]);

        rt.expect_validate_caller_type(vec![*MINER_ACTOR_CODE_ID]);
        rt.set_caller(*MINER_ACTOR_CODE_ID, PROVIDER_ADDR);
        let params = ActivateDealsParams { deal_ids: vec![deal_id], sector_expiry };
        expect_abort(
            ExitCode::USR_ILLEGAL_ARGUMENT,
            rt.call::<MarketActor>(
                Method::ActivateDeals as u64,
                &RawBytes::serialize(params).unwrap(),
            ),
        );

        rt.verify();
        check_state(&rt);
    }
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
    assert_eq!(TokenAmount::from(0u8), pay);
    assert_eq!(TokenAmount::from(0u8), slashed);

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
        *BURNT_FUNDS_ACTOR_ADDR,
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
    let sig = Signature::new_bls("does not matter".as_bytes().to_vec());
    let params = PublishStorageDealsParams {
        deals: vec![ClientDealProposal { proposal: d2.clone(), client_signature: sig.clone() }],
    };
    rt.expect_validate_caller_type(vec![*ACCOUNT_ACTOR_CODE_ID, *MULTISIG_ACTOR_CODE_ID]);
    expect_provider_control_address(&mut rt, PROVIDER_ADDR, OWNER_ADDR, WORKER_ADDR);
    expect_query_network_info(&mut rt);
    rt.set_caller(*ACCOUNT_ACTOR_CODE_ID, WORKER_ADDR);
    rt.expect_verify_signature(ExpectedVerifySig {
        sig,
        signer: d2.client,
        plaintext: buf.to_vec(),
        result: Ok(()),
    });
    expect_abort(
        ExitCode::USR_ILLEGAL_ARGUMENT,
        rt.call::<MarketActor>(
            Method::PublishStorageDeals as u64,
            &RawBytes::serialize(params).unwrap(),
        ),
    );
    rt.verify();
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

    rt.expect_validate_caller_type(vec![*MINER_ACTOR_CODE_ID]);
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

    rt.expect_validate_caller_type(vec![*MINER_ACTOR_CODE_ID]);
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

    rt.expect_validate_caller_type(vec![*MINER_ACTOR_CODE_ID]);
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
