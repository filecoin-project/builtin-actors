// Copyright 2019-2022 ChainSafe Systems
// SPDX-License-Identifier: Apache-2.0, MIT

use std::collections::HashMap;

use fil_actor_market::balance_table::{BalanceTable, BALANCE_TABLE_BITWIDTH};
use fil_actor_market::{
    ext, Actor as MarketActor, Label, Method, State, WithdrawBalanceParams, PROPOSALS_AMT_BITWIDTH,
    STATES_AMT_BITWIDTH,
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
    let sv_bz = to_vec(&label)
        .map_err(|e| ActorError::from(e).wrap("failed to serialize DealProposal"))
        .unwrap();
    println!("{:?}", sv_bz);

    let label2 = Label::Bytes(b"i_am_random_____i_am_random_____".to_vec());
    println!("{:?}", (b"i_am_random_____i_am_random_____".to_vec()));
    let sv_bz = to_vec(&label2)
        .map_err(|e| ActorError::from(e).wrap("failed to serialize DealProposal"))
        .unwrap();
    println!("{:?}", sv_bz);
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

#[ignore]
#[test]
fn add_provider_escrow_funds() {
    // First element of tuple is the delta the second element is the total after the delta change
    let test_cases = vec![(10, 10), (20, 30), (40, 70)];

    let owner_addr = Address::new_id(OWNER_ID);
    let worker_addr = Address::new_id(WORKER_ID);
    let provider_addr = Address::new_id(PROVIDER_ID);

    for caller_addr in &[owner_addr, worker_addr] {
        let mut rt = setup();

        for test_case in test_cases.clone() {
            rt.set_caller(*ACCOUNT_ACTOR_CODE_ID, *caller_addr);

            let amount = TokenAmount::from(test_case.0 as u64);
            rt.set_value(amount);

            expect_provider_control_address(&mut rt, provider_addr, owner_addr, worker_addr);

            assert!(rt
                .call::<MarketActor>(
                    Method::AddBalance as u64,
                    &RawBytes::serialize(provider_addr).unwrap(),
                )
                .is_ok());
            rt.verify();

            assert_eq!(
                get_escrow_balance(&rt, &provider_addr).unwrap(),
                TokenAmount::from(test_case.1 as u64)
            );
        }
    }
}

#[ignore]
#[test]
fn account_actor_check() {
    let mut rt = setup();

    let amount = TokenAmount::from(10u8);
    rt.set_value(amount);

    let owner_addr = Address::new_id(OWNER_ID);
    let worker_addr = Address::new_id(WORKER_ID);
    let provider_addr = Address::new_id(PROVIDER_ID);

    expect_provider_control_address(&mut rt, provider_addr, owner_addr, worker_addr);
    rt.set_caller(*MINER_ACTOR_CODE_ID, provider_addr);

    assert_eq!(
        ExitCode::ErrForbidden,
        rt.call::<MarketActor>(
            Method::AddBalance as u64,
            &RawBytes::serialize(provider_addr).unwrap(),
        )
        .unwrap_err()
        .exit_code()
    );

    rt.verify();
}

#[ignore]
#[test]
fn add_non_provider_funds() {
    // First element of tuple is the delta the second element is the total after the delta change
    let test_cases = vec![(10, 10), (20, 30), (40, 70)];

    let client_addr = Address::new_id(CLIENT_ID);
    let worker_addr = Address::new_id(WORKER_ID);

    for caller_addr in &[client_addr, worker_addr] {
        let mut rt = setup();

        for test_case in test_cases.clone() {
            rt.set_caller(*ACCOUNT_ACTOR_CODE_ID, *caller_addr);

            let amount = TokenAmount::from(test_case.0 as u64);
            rt.set_value(amount);
            rt.expect_validate_caller_type((*CALLER_TYPES_SIGNABLE).clone());

            assert!(rt
                .call::<MarketActor>(
                    Method::AddBalance as u64,
                    &RawBytes::serialize(*caller_addr).unwrap(),
                )
                .is_ok());

            rt.verify();

            assert_eq!(
                get_escrow_balance(&rt, caller_addr).unwrap(),
                TokenAmount::from(test_case.1 as u8)
            );
        }
    }
}

#[ignore]
#[test]
fn withdraw_provider_to_owner() {
    let mut rt = setup();

    let owner_addr = Address::new_id(OWNER_ID);
    let worker_addr = Address::new_id(WORKER_ID);
    let provider_addr = Address::new_id(PROVIDER_ID);

    let amount = TokenAmount::from(20u8);
    add_provider_funds(&mut rt, provider_addr, owner_addr, worker_addr, amount.clone());

    assert_eq!(amount, get_escrow_balance(&rt, &provider_addr).unwrap());

    rt.set_caller(*ACCOUNT_ACTOR_CODE_ID, worker_addr);
    expect_provider_control_address(&mut rt, provider_addr, owner_addr, worker_addr);

    let withdraw_amount = TokenAmount::from(1u8);

    rt.expect_send(
        owner_addr,
        METHOD_SEND,
        RawBytes::default(),
        withdraw_amount.clone(),
        RawBytes::default(),
        ExitCode::Ok,
    );

    let params =
        WithdrawBalanceParams { provider_or_client: provider_addr, amount: withdraw_amount };

    assert!(rt
        .call::<MarketActor>(Method::WithdrawBalance as u64, &RawBytes::serialize(params).unwrap(),)
        .is_ok());

    rt.verify();

    assert_eq!(get_escrow_balance(&rt, &provider_addr).unwrap(), TokenAmount::from(19u8));
}

#[ignore]
#[test]
fn withdraw_non_provider() {
    // Test is currently failing because curr_epoch  is 0. When subtracted by 1, it goe snmegative causing a overflow error
    let mut rt = setup();

    let client_addr = Address::new_id(CLIENT_ID);

    let amount = TokenAmount::from(20u8);
    add_participant_funds(&mut rt, client_addr, amount.clone());

    assert_eq!(amount, get_escrow_balance(&rt, &client_addr).unwrap());

    rt.set_caller(*ACCOUNT_ACTOR_CODE_ID, client_addr);
    rt.expect_validate_caller_type(vec![*ACCOUNT_ACTOR_CODE_ID, *MULTISIG_ACTOR_CODE_ID]);

    let withdraw_amount = TokenAmount::from(1u8);

    rt.expect_send(
        client_addr,
        METHOD_SEND,
        RawBytes::default(),
        withdraw_amount.clone(),
        RawBytes::default(),
        ExitCode::Ok,
    );

    let params = WithdrawBalanceParams { provider_or_client: client_addr, amount: withdraw_amount };

    assert!(rt
        .call::<MarketActor>(Method::WithdrawBalance as u64, &RawBytes::serialize(params).unwrap(),)
        .is_ok());

    rt.verify();

    assert_eq!(get_escrow_balance(&rt, &client_addr).unwrap(), TokenAmount::from(19u8));
}

#[ignore]
#[test]
fn client_withdraw_more_than_available() {
    let mut rt = setup();

    let client_addr = Address::new_id(CLIENT_ID);

    let amount = TokenAmount::from(20u8);
    add_participant_funds(&mut rt, client_addr, amount.clone());

    rt.set_caller(*ACCOUNT_ACTOR_CODE_ID, client_addr);
    rt.expect_validate_caller_type(vec![*ACCOUNT_ACTOR_CODE_ID, *MULTISIG_ACTOR_CODE_ID]);

    let withdraw_amount = TokenAmount::from(25u8);

    rt.expect_send(
        client_addr,
        METHOD_SEND,
        RawBytes::default(),
        amount,
        RawBytes::default(),
        ExitCode::Ok,
    );

    let params = WithdrawBalanceParams { provider_or_client: client_addr, amount: withdraw_amount };

    assert!(rt
        .call::<MarketActor>(Method::WithdrawBalance as u64, &RawBytes::serialize(params).unwrap(),)
        .is_ok());

    rt.verify();

    assert_eq!(get_escrow_balance(&rt, &client_addr).unwrap(), TokenAmount::from(0u8));
}

#[ignore]
#[test]
fn worker_withdraw_more_than_available() {
    let mut rt = setup();

    let owner_addr = Address::new_id(OWNER_ID);
    let worker_addr = Address::new_id(WORKER_ID);
    let provider_addr = Address::new_id(PROVIDER_ID);

    let amount = TokenAmount::from(20u8);
    add_provider_funds(&mut rt, provider_addr, owner_addr, worker_addr, amount.clone());

    assert_eq!(amount, get_escrow_balance(&rt, &provider_addr).unwrap());

    rt.set_caller(*ACCOUNT_ACTOR_CODE_ID, worker_addr);
    expect_provider_control_address(&mut rt, provider_addr, owner_addr, worker_addr);

    let withdraw_amount = TokenAmount::from(25u8);

    rt.expect_send(
        owner_addr,
        METHOD_SEND,
        RawBytes::default(),
        amount,
        RawBytes::default(),
        ExitCode::Ok,
    );

    let params =
        WithdrawBalanceParams { provider_or_client: provider_addr, amount: withdraw_amount };

    assert!(rt
        .call::<MarketActor>(Method::WithdrawBalance as u64, &RawBytes::serialize(params).unwrap(),)
        .is_ok());

    rt.verify();

    assert_eq!(get_escrow_balance(&rt, &provider_addr).unwrap(), TokenAmount::from(0u8));
}

fn expect_provider_control_address(
    rt: &mut MockRuntime,
    provider: Address,
    owner: Address,
    worker: Address,
) {
    rt.expect_validate_caller_addr(vec![owner, worker]);

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
        ExitCode::Ok,
    );
}

fn add_provider_funds(
    rt: &mut MockRuntime,
    provider: Address,
    owner: Address,
    worker: Address,
    amount: TokenAmount,
) {
    rt.set_value(amount.clone());

    rt.set_caller(*ACCOUNT_ACTOR_CODE_ID, owner);
    expect_provider_control_address(rt, provider, owner, worker);

    assert!(rt
        .call::<MarketActor>(Method::AddBalance as u64, &RawBytes::serialize(provider).unwrap(),)
        .is_ok());

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
