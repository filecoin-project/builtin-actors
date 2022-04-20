// Copyright 2019-2022 ChainSafe Systems
// SPDX-License-Identifier: Apache-2.0, MIT

use std::collections::HashMap;

use fil_actor_market::balance_table::{BalanceTable, BALANCE_TABLE_BITWIDTH};
use fil_actor_market::{
    ext, Actor as MarketActor, Label, Method, State, WithdrawBalanceParams, WithdrawBalanceReturn,
    PROPOSALS_AMT_BITWIDTH, STATES_AMT_BITWIDTH,
};
use fil_actors_runtime::cbor::deserialize;
use fil_actors_runtime::runtime::Runtime;
use fil_actors_runtime::test_utils::*;
use fil_actors_runtime::{
    make_empty_map, ActorError, SetMultimap, STORAGE_MARKET_ACTOR_ADDR, SYSTEM_ACTOR_ADDR,
};
use fvm_ipld_amt::Amt;
use fvm_ipld_encoding::{to_vec, RawBytes};
use fvm_shared::address::Address;
use fvm_shared::bigint::bigint_ser::BigIntDe;
use fvm_shared::clock::EPOCH_UNDEFINED;
use fvm_shared::econ::TokenAmount;
use fvm_shared::error::ExitCode;
use fvm_shared::{HAMT_BIT_WIDTH, METHOD_CONSTRUCTOR, METHOD_SEND};

const OWNER_ID: u64 = 101;
const PROVIDER_ID: u64 = 102;
const WORKER_ID: u64 = 103;
const CLIENT_ID: u64 = 104;

fn setup() -> MockRuntime {
    let mut actor_code_cids = HashMap::default();
    actor_code_cids.insert(Address::new_id(OWNER_ID), *ACCOUNT_ACTOR_CODE_ID);
    actor_code_cids.insert(Address::new_id(WORKER_ID), *ACCOUNT_ACTOR_CODE_ID);
    actor_code_cids.insert(Address::new_id(PROVIDER_ID), *MINER_ACTOR_CODE_ID);
    actor_code_cids.insert(Address::new_id(CLIENT_ID), *ACCOUNT_ACTOR_CODE_ID);

    let mut rt = MockRuntime {
        receiver: *STORAGE_MARKET_ACTOR_ADDR,
        caller: *SYSTEM_ACTOR_ADDR,
        caller_type: *INIT_ACTOR_CODE_ID,
        actor_code_cids,
        ..Default::default()
    };
    construct_and_verify(&mut rt);

    rt
}

fn get_escrow_balance(rt: &MockRuntime, addr: &Address) -> Result<TokenAmount, ActorError> {
    let st: State = rt.get_state()?;

    let et = BalanceTable::from_root(rt.store(), &st.escrow_table).unwrap();

    Ok(et.get(addr).unwrap())
}

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

    let state_data: State = rt.get_state().unwrap();

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

    let owner = Address::new_id(OWNER_ID);
    let worker = Address::new_id(WORKER_ID);
    let provider = Address::new_id(PROVIDER_ID);

    for caller_addr in &[owner, worker] {
        let mut rt = setup();

        for tc in &test_cases {
            rt.set_caller(*ACCOUNT_ACTOR_CODE_ID, *caller_addr);
            rt.set_value(TokenAmount::from(tc.delta));
            rt.expect_validate_caller_type((*CALLER_TYPES_SIGNABLE).clone());
            expect_provider_control_address(&mut rt, provider, owner, worker);

            assert_eq!(
                RawBytes::default(),
                rt.call::<MarketActor>(
                    Method::AddBalance as u64,
                    &RawBytes::serialize(provider).unwrap(),
                )
                .unwrap()
            );

            rt.verify();

            assert_eq!(get_escrow_balance(&rt, &provider).unwrap(), TokenAmount::from(tc.total));
            // TODO: actor.checkState(rt)
        }
    }
}

#[test]
fn fails_unless_called_by_an_account_actor() {
    let mut rt = setup();

    rt.set_value(TokenAmount::from(10));
    rt.expect_validate_caller_type((*CALLER_TYPES_SIGNABLE).clone());

    let provider_addr = Address::new_id(PROVIDER_ID);
    rt.set_caller(*MINER_ACTOR_CODE_ID, provider_addr);
    assert_eq!(
        ExitCode::USR_FORBIDDEN,
        rt.call::<MarketActor>(
            Method::AddBalance as u64,
            &RawBytes::serialize(provider_addr).unwrap(),
        )
        .unwrap_err()
        .exit_code()
    );

    rt.verify();
    // TODO: actor.checkState(rt)
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

    let client = Address::new_id(CLIENT_ID);
    let worker = Address::new_id(WORKER_ID);

    for caller_addr in &[client, worker] {
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
            // TODO: actor.checkState(rt)
        }
    }
}

#[test]
fn withdraws_from_provider_escrow_funds_and_sends_to_owner() {
    let mut rt = setup();

    let provider_addr = Address::new_id(PROVIDER_ID);
    let owner_addr = Address::new_id(OWNER_ID);
    let worker_addr = Address::new_id(WORKER_ID);

    let amount = TokenAmount::from(20);
    add_provider_funds(&mut rt, amount.clone(), provider_addr, owner_addr, worker_addr);

    assert_eq!(amount, get_escrow_balance(&rt, &provider_addr).unwrap());

    // worker calls WithdrawBalance, balance is transferred to owner
    let withdraw_amount = TokenAmount::from(1);
    withdraw_provider_balance(
        &mut rt,
        withdraw_amount.clone(),
        withdraw_amount,
        provider_addr,
        owner_addr,
        worker_addr,
    );

    assert_eq!(TokenAmount::from(19), get_escrow_balance(&rt, &provider_addr).unwrap());
    // TODO: actor.checkState(rt)
}

#[test]
fn withdraws_from_non_provider_escrow_funds() {
    let mut rt = setup();

    let client_addr = Address::new_id(CLIENT_ID);

    let amount = TokenAmount::from(20);
    add_participant_funds(&mut rt, client_addr, amount.clone());

    assert_eq!(get_escrow_balance(&rt, &client_addr).unwrap(), amount);

    let withdraw_amount = TokenAmount::from(1);
    withdraw_client_balance(&mut rt, withdraw_amount.clone(), withdraw_amount, client_addr);

    assert_eq!(get_escrow_balance(&rt, &client_addr).unwrap(), TokenAmount::from(19));
    // TODO: actor.checkState(rt)
}

#[test]
fn client_withdrawing_more_than_escrow_balance_limits_to_available_funds() {
    let mut rt = setup();

    let client_addr = Address::new_id(CLIENT_ID);

    let amount = TokenAmount::from(20);
    add_participant_funds(&mut rt, client_addr, amount.clone());

    // withdraw amount greater than escrow balance
    let withdraw_amount = TokenAmount::from(25);
    withdraw_client_balance(&mut rt, withdraw_amount, amount, client_addr);

    assert_eq!(get_escrow_balance(&rt, &client_addr).unwrap(), TokenAmount::from(0));
}

#[test]
fn worker_withdrawing_more_than_escrow_balance_limits_to_available_funds() {
    let mut rt = setup();

    let provider_addr = Address::new_id(PROVIDER_ID);
    let owner_addr = Address::new_id(OWNER_ID);
    let worker_addr = Address::new_id(WORKER_ID);

    let amount = TokenAmount::from(20);
    add_provider_funds(&mut rt, amount.clone(), provider_addr, owner_addr, worker_addr);

    assert_eq!(get_escrow_balance(&rt, &provider_addr).unwrap(), amount);

    let withdraw_amount = TokenAmount::from(25);
    withdraw_provider_balance(
        &mut rt,
        withdraw_amount,
        amount,
        provider_addr,
        owner_addr,
        worker_addr,
    );

    assert_eq!(get_escrow_balance(&rt, &provider_addr).unwrap(), TokenAmount::from(0));
    // TODO: actor.checkState(rt)
}

fn expect_provider_control_address(
    rt: &mut MockRuntime,
    provider: Address,
    owner: Address,
    worker: Address,
) {
    //rt.expect_validate_caller_addr(vec![owner, worker]);
    let return_value = ext::miner::GetControlAddressesReturnParams {
        owner,
        worker,
        control_addresses: Vec::new(),
    };

    rt.expect_send(
        provider,
        ext::miner::CONTROL_ADDRESSES_METHOD,
        RawBytes::default(),
        TokenAmount::from(0u8),
        RawBytes::serialize(return_value).unwrap(),
        ExitCode::OK,
    );
}

fn add_provider_funds(
    rt: &mut MockRuntime,
    amount: TokenAmount,
    provider: Address,
    owner: Address,
    worker: Address,
) {
    rt.set_value(amount.clone());
    rt.set_address_actor_type(provider, *MINER_ACTOR_CODE_ID);
    rt.set_caller(*ACCOUNT_ACTOR_CODE_ID, owner);
    rt.expect_validate_caller_type((*CALLER_TYPES_SIGNABLE).clone());

    expect_provider_control_address(rt, provider, owner, worker);

    assert_eq!(
        RawBytes::default(),
        rt.call::<MarketActor>(Method::AddBalance as u64, &RawBytes::serialize(provider).unwrap(),)
            .unwrap()
    );
    rt.verify();
    rt.add_balance(amount);
}

fn add_participant_funds(rt: &mut MockRuntime, addr: Address, amount: TokenAmount) {
    rt.set_value(amount.clone());

    rt.set_caller(*ACCOUNT_ACTOR_CODE_ID, addr);

    rt.expect_validate_caller_type(vec![*ACCOUNT_ACTOR_CODE_ID, *MULTISIG_ACTOR_CODE_ID]);

    assert!(rt
        .call::<MarketActor>(Method::AddBalance as u64, &RawBytes::serialize(addr).unwrap(),)
        .is_ok());

    rt.verify();

    rt.add_balance(amount);
}

fn construct_and_verify(rt: &mut MockRuntime) {
    rt.expect_validate_caller_addr(vec![*SYSTEM_ACTOR_ADDR]);
    assert_eq!(
        RawBytes::default(),
        rt.call::<MarketActor>(METHOD_CONSTRUCTOR, &RawBytes::default(),).unwrap()
    );
    rt.verify();
}

fn withdraw_provider_balance(
    rt: &mut MockRuntime,
    withdraw_amount: TokenAmount,
    expected_send: TokenAmount,
    provider: Address,
    owner: Address,
    worker: Address,
) {
    rt.set_caller(*ACCOUNT_ACTOR_CODE_ID, worker);
    rt.expect_validate_caller_addr(vec![owner, worker]);
    expect_provider_control_address(rt, provider, owner, worker);

    let params = WithdrawBalanceParams { provider_or_client: provider, amount: withdraw_amount };

    rt.expect_send(
        owner,
        METHOD_SEND,
        RawBytes::default(),
        expected_send.clone(),
        RawBytes::default(),
        ExitCode::OK,
    );
    let ret: WithdrawBalanceReturn = rt
        .call::<MarketActor>(Method::WithdrawBalance as u64, &RawBytes::serialize(params).unwrap())
        .unwrap()
        .deserialize()
        .unwrap();
    rt.verify();

    assert_eq!(
        expected_send, ret.amount_withdrawn,
        "return value indicates {} withdrawn but expected {}",
        ret.amount_withdrawn, expected_send
    );
}

fn withdraw_client_balance(
    rt: &mut MockRuntime,
    withdraw_amount: TokenAmount,
    expected_send: TokenAmount,
    client: Address,
) {
    rt.set_caller(*ACCOUNT_ACTOR_CODE_ID, client);
    rt.expect_send(
        client,
        METHOD_SEND,
        RawBytes::default(),
        expected_send.clone(),
        RawBytes::default(),
        ExitCode::OK,
    );
    rt.expect_validate_caller_addr(vec![client]);

    let params = WithdrawBalanceParams { provider_or_client: client, amount: withdraw_amount };

    let ret: WithdrawBalanceReturn = rt
        .call::<MarketActor>(Method::WithdrawBalance as u64, &RawBytes::serialize(params).unwrap())
        .unwrap()
        .deserialize()
        .unwrap();
    rt.verify();

    assert_eq!(
        expected_send, ret.amount_withdrawn,
        "return value indicates {} withdrawn but expected {}",
        ret.amount_withdrawn, expected_send
    );
}
