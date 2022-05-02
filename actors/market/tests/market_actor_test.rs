// Copyright 2019-2022 ChainSafe Systems
// SPDX-License-Identifier: Apache-2.0, MIT

use std::collections::HashMap;
use std::convert::TryInto;

use fil_actor_market::balance_table::{BalanceTable, BALANCE_TABLE_BITWIDTH};
use fil_actor_market::ext::miner::GetControlAddressesReturnParams;
use fil_actor_market::{
    ext, gen_rand_next_epoch, ActivateDealsParams, Actor as MarketActor, ClientDealProposal,
    DealArray, DealMetaArray, DealProposal, DealState, Label, Method,
    OnMinerSectorsTerminateParams, PublishStorageDealsParams, PublishStorageDealsReturn, State,
    WithdrawBalanceParams, WithdrawBalanceReturn, PROPOSALS_AMT_BITWIDTH, STATES_AMT_BITWIDTH,
};
use fil_actor_power::{CurrentTotalPowerReturn, Method as PowerMethod};
use fil_actor_reward::Method as RewardMethod;
use fil_actor_verifreg::UseBytesParams;
use fil_actors_runtime::cbor::deserialize;
use fil_actors_runtime::network::EPOCHS_IN_DAY;
use fil_actors_runtime::runtime::{Policy, Runtime};
use fil_actors_runtime::test_utils::*;
use fil_actors_runtime::{
    make_empty_map, ActorError, SetMultimap, BURNT_FUNDS_ACTOR_ADDR, CRON_ACTOR_ADDR,
    REWARD_ACTOR_ADDR, STORAGE_MARKET_ACTOR_ADDR, STORAGE_POWER_ACTOR_ADDR, SYSTEM_ACTOR_ADDR,
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
use fvm_shared::reward::ThisEpochRewardReturn;
use fvm_shared::sector::StoragePower;
use fvm_shared::smooth::FilterEstimate;
use fvm_shared::{HAMT_BIT_WIDTH, METHOD_CONSTRUCTOR, METHOD_SEND};

use cid::Cid;
use num_traits::FromPrimitive;

const OWNER_ID: u64 = 101;
const PROVIDER_ID: u64 = 102;
const WORKER_ID: u64 = 103;
const CLIENT_ID: u64 = 104;
const CONTROL_ID: u64 = 200;

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
    let st: State = rt.get_state();

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
fn fails_if_withdraw_from_non_provider_funds_is_not_initiated_by_the_recipient() {
    let mut rt = setup();
    let client = Address::new_id(CLIENT_ID);

    add_participant_funds(&mut rt, client, TokenAmount::from(20u8));

    assert_eq!(TokenAmount::from(20u8), get_escrow_balance(&rt, &client).unwrap());

    rt.expect_validate_caller_addr(vec![client]);

    let params =
        WithdrawBalanceParams { provider_or_client: client, amount: TokenAmount::from(1u8) };

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
    assert_eq!(TokenAmount::from(20u8), get_escrow_balance(&rt, &client).unwrap());

    // TODO: actor.checkState(rt)
}

#[test]
fn balance_after_withdrawal_must_always_be_greater_than_or_equal_to_locked_amount() {
    let mut rt = setup();
    let publish_epoch = ChainEpoch::from(5);
    rt.set_epoch(publish_epoch);

    let client = Address::new_id(CLIENT_ID);
    let worker = Address::new_id(WORKER_ID);
    let provider = Address::new_id(PROVIDER_ID);
    let owner = Address::new_id(OWNER_ID);
    let control = Address::new_id(CONTROL_ID);
    let start_epoch = ChainEpoch::from(10);
    let end_epoch = start_epoch + 200 * EPOCHS_IN_DAY;

    let deal_id = generate_and_publish_deal(
        &mut rt,
        client,
        provider,
        owner,
        worker,
        control,
        start_epoch,
        end_epoch,
    );

    let deal = get_deal_proposal(&mut rt, deal_id);

    assert_eq!(deal.provider_collateral, get_escrow_balance(&rt, &provider).unwrap());
    assert_eq!(deal.client_balance_requirement(), get_escrow_balance(&rt, &client).unwrap());

    let withdraw_amount = TokenAmount::from(1u8);
    let withdrawable_amount = TokenAmount::from(0u8);

    withdraw_client_balance(&mut rt, withdraw_amount.clone(), withdrawable_amount.clone(), client);
    withdraw_provider_balance(
        &mut rt,
        withdraw_amount,
        withdrawable_amount,
        provider,
        owner,
        worker,
    );

    let withdraw_amount = TokenAmount::from(30u8);
    let withdrawable_amount = TokenAmount::from(25u8);

    add_provider_funds(&mut rt, withdrawable_amount.clone(), provider, owner, worker);
    withdraw_provider_balance(
        &mut rt,
        withdraw_amount.clone(),
        withdrawable_amount.clone(),
        provider,
        owner,
        worker,
    );

    add_participant_funds(&mut rt, client, withdrawable_amount.clone());
    withdraw_client_balance(&mut rt, withdraw_amount, withdrawable_amount, client);
    // TODO: actor.checkState(rt)
}

#[test]
fn worker_balance_after_withdrawal_must_account_for_slashed_funds() {
    let client = Address::new_id(CLIENT_ID);
    let worker = Address::new_id(WORKER_ID);
    let provider = Address::new_id(PROVIDER_ID);
    let owner = Address::new_id(OWNER_ID);
    let control = Address::new_id(CONTROL_ID);
    let start_epoch = ChainEpoch::from(10);
    let end_epoch = start_epoch + 200 * EPOCHS_IN_DAY;
    let publish_epoch = ChainEpoch::from(5);

    let mut rt = setup();

    // publish deal
    rt.set_epoch(publish_epoch);
    let deal_id = generate_and_publish_deal(
        &mut rt,
        client,
        provider,
        owner,
        worker,
        control,
        start_epoch,
        end_epoch,
    );

    // activate the deal
    activate_deals(&mut rt, end_epoch + 1, provider, publish_epoch, &[deal_id]);
    let st = get_deal_state(&mut rt, deal_id);
    assert_eq!(publish_epoch, st.sector_start_epoch);

    // slash the deal
    rt.set_epoch(publish_epoch + 1);
    terminate_deals(&mut rt, provider, &[deal_id]);
    let st = get_deal_state(&mut rt, deal_id);
    assert_eq!(publish_epoch + 1, st.slash_epoch);

    // provider cannot withdraw any funds since all it's balance is locked
    let withdraw_amount = TokenAmount::from(1);
    let actual_withdrawn = TokenAmount::from(0);
    withdraw_provider_balance(&mut rt, withdraw_amount, actual_withdrawn, provider, owner, worker);

    // add some more funds to the provider & ensure withdrawal is limited by the locked funds
    add_provider_funds(&mut rt, TokenAmount::from(25), provider, owner, worker);
    let withdraw_amount = TokenAmount::from(30);
    let actual_withdrawn = TokenAmount::from(25);

    withdraw_provider_balance(&mut rt, withdraw_amount, actual_withdrawn, provider, owner, worker);
    // TODO: actor.checkState(rt)
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

#[test]
fn fail_when_balance_is_zero() {
    let mut rt = setup();

    let provider = Address::new_id(PROVIDER_ID);

    rt.set_caller(*ACCOUNT_ACTOR_CODE_ID, Address::new_id(OWNER_ID));
    rt.set_received(BigInt::from(0_i32));

    expect_abort(
        ExitCode::USR_ILLEGAL_ARGUMENT,
        rt.call::<MarketActor>(Method::AddBalance as u64, &RawBytes::serialize(&provider).unwrap()),
    );

    rt.verify();
}

#[test]
fn fails_with_a_negative_withdraw_amount() {
    let mut rt = setup();

    let params = WithdrawBalanceParams {
        provider_or_client: Address::new_id(PROVIDER_ID),
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

    let owner_addr = Address::new_id(OWNER_ID);
    let worker_addr = Address::new_id(WORKER_ID);
    let provider_addr = Address::new_id(PROVIDER_ID);

    let amount = TokenAmount::from(20u8);
    add_provider_funds(&mut rt, amount.clone(), provider_addr, owner_addr, worker_addr);

    assert_eq!(get_escrow_balance(&rt, &provider_addr).unwrap(), amount);

    // only signing parties can add balance for client AND provider.
    rt.expect_validate_caller_addr(vec![owner_addr, worker_addr]);
    let params =
        WithdrawBalanceParams { provider_or_client: provider_addr, amount: TokenAmount::from(1u8) };

    // caller is not owner or worker
    rt.set_caller(*ACCOUNT_ACTOR_CODE_ID, Address::new_id(909));
    expect_get_control_addresses(&mut rt, provider_addr, owner_addr, worker_addr, vec![]);

    expect_abort(
        ExitCode::USR_FORBIDDEN,
        rt.call::<MarketActor>(
            Method::WithdrawBalance as u64,
            &RawBytes::serialize(&params).unwrap(),
        ),
    );
    rt.verify();

    // verify there was no withdrawal
    assert_eq!(get_escrow_balance(&rt, &provider_addr).unwrap(), amount);
    // TODO: actor.checkState(rt)
}

fn expect_get_control_addresses(
    rt: &mut MockRuntime,
    provider: Address,
    owner: Address,
    worker: Address,
    controls: Vec<Address>,
) {
    let result = GetControlAddressesReturnParams { owner, worker, control_addresses: controls };

    rt.expect_send(
        provider,
        ext::miner::CONTROL_ADDRESSES_METHOD,
        RawBytes::default(),
        BigInt::from(0u8),
        RawBytes::serialize(result).unwrap(),
        ExitCode::OK,
    )
}

#[test]
fn deal_starts_on_day_boundary() {
    let deal_updates_interval = Policy::default().deal_updates_interval;
    let start_epoch = deal_updates_interval; // 2880
    let end_epoch = start_epoch + 200 * EPOCHS_IN_DAY;
    let publish_epoch = ChainEpoch::from(1);

    let mut rt = setup();
    rt.set_epoch(publish_epoch);

    let client_addr = Address::new_id(CLIENT_ID);
    let provider_addr = Address::new_id(PROVIDER_ID);
    let owner_addr = Address::new_id(OWNER_ID);
    let worker_addr = Address::new_id(WORKER_ID);
    let control_addr = Address::new_id(CONTROL_ID);

    for i in 0..(3 * deal_updates_interval) {
        let piece_cid = make_piece_cid((format!("{i}")).as_bytes());
        let deal_id = generate_and_publish_deal_for_piece(
            &mut rt,
            client_addr,
            provider_addr,
            owner_addr,
            worker_addr,
            control_addr,
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

    let client_addr = Address::new_id(CLIENT_ID);
    let provider_addr = Address::new_id(PROVIDER_ID);
    let owner_addr = Address::new_id(OWNER_ID);
    let worker_addr = Address::new_id(WORKER_ID);
    let control_addr = Address::new_id(CONTROL_ID);

    // First 1000 deals (start_epoch % update interval) scheduled starting in the next day
    for i in 0..1000 {
        let piece_cid = make_piece_cid((format!("{i}")).as_bytes());
        let deal_id = generate_and_publish_deal_for_piece(
            &mut rt,
            client_addr,
            provider_addr,
            owner_addr,
            worker_addr,
            control_addr,
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
            client_addr,
            provider_addr,
            owner_addr,
            worker_addr,
            control_addr,
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

    let owner_addr = Address::new_id(OWNER_ID);
    let provider_addr = Address::new_id(PROVIDER_ID);
    let worker_addr = Address::new_id(WORKER_ID);
    let client_addr = Address::new_id(CLIENT_ID);
    let control_addr = Address::new_id(CONTROL_ID);

    // Publish from miner worker.
    let deal1 = generate_deal_and_add_funds(
        &mut rt,
        client_addr,
        provider_addr,
        owner_addr,
        worker_addr,
        start_epoch,
        end_epoch,
    );
    rt.set_caller(*ACCOUNT_ACTOR_CODE_ID, worker_addr);
    publish_deals(&mut rt, provider_addr, owner_addr, worker_addr, control_addr, &[deal1]);

    // Publish from miner control address.
    let deal2 = generate_deal_and_add_funds(
        &mut rt,
        client_addr,
        provider_addr,
        owner_addr,
        worker_addr,
        start_epoch + 1,
        end_epoch + 1,
    );
    rt.set_caller(*ACCOUNT_ACTOR_CODE_ID, control_addr);
    publish_deals(&mut rt, provider_addr, owner_addr, worker_addr, control_addr, &[deal2]);
    // TODO: actor.checkState(rt)
}

// Converted from: https://github.com/filecoin-project/specs-actors/blob/0afe155bfffa036057af5519afdead845e0780de/actors/builtin/market/market_test.go#L529
#[test]
fn provider_and_client_addresses_are_resolved_before_persisting_state_and_sent_to_verigreg_actor_for_a_verified_deal(
) {
    use fvm_shared::address::BLS_PUB_LEN;
    let owner_addr = Address::new_id(OWNER_ID);
    let worker_addr = Address::new_id(WORKER_ID);

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
    rt.set_caller(*ACCOUNT_ACTOR_CODE_ID, owner_addr);
    rt.expect_validate_caller_type((*CALLER_TYPES_SIGNABLE).clone());
    expect_get_control_addresses(&mut rt, provider_resolved, owner_addr, worker_addr, vec![]);

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
    rt.set_caller(*ACCOUNT_ACTOR_CODE_ID, worker_addr);
    rt.expect_validate_caller_type((*CALLER_TYPES_SIGNABLE).clone());

    expect_get_control_addresses(&mut rt, provider_resolved, owner_addr, worker_addr, vec![]);
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

    // TODO actor.checkState(rt)
}

#[test]
fn publish_a_deal_after_activating_a_previous_deal_which_has_a_start_epoch_far_in_the_future() {
    let start_epoch = 1000;
    let end_epoch = start_epoch + 200 * EPOCHS_IN_DAY;
    let publish_epoch = ChainEpoch::from(1);

    let owner_addr = Address::new_id(OWNER_ID);
    let provider_addr = Address::new_id(PROVIDER_ID);
    let worker_addr = Address::new_id(WORKER_ID);
    let client_addr = Address::new_id(CLIENT_ID);
    let control_addr = Address::new_id(CONTROL_ID);

    let mut rt = setup();

    // publish the deal and activate it
    rt.set_epoch(publish_epoch);
    let deal1 = generate_and_publish_deal(
        &mut rt,
        client_addr,
        provider_addr,
        owner_addr,
        worker_addr,
        control_addr,
        start_epoch,
        end_epoch,
    );
    activate_deals(&mut rt, end_epoch, provider_addr, publish_epoch, &[deal1]);
    let st = get_deal_state(&mut rt, deal1);
    assert_eq!(publish_epoch, st.sector_start_epoch);

    // now publish a second deal and activate it
    let new_epoch = publish_epoch + 1;
    rt.set_epoch(new_epoch);
    let deal2 = generate_and_publish_deal(
        &mut rt,
        client_addr,
        provider_addr,
        owner_addr,
        worker_addr,
        control_addr,
        start_epoch + 1,
        end_epoch + 1,
    );
    activate_deals(&mut rt, end_epoch + 1, provider_addr, new_epoch, &[deal2]);
    // TODO: actor.checkState(rt)
}

// Converted from https://github.com/filecoin-project/specs-actors/blob/d56b240af24517443ce1f8abfbdab7cb22d331f1/actors/builtin/market/market_test.go#L1274
#[test]
fn terminate_multiple_deals_from_multiple_providers() {
    let start_epoch = 10;
    let end_epoch = start_epoch + 200 * EPOCHS_IN_DAY;
    let sector_expiry = end_epoch + 100;
    let current_epoch = 5;
    let owner_addr = Address::new_id(OWNER_ID);
    let provider_addr = Address::new_id(PROVIDER_ID);
    let worker_addr = Address::new_id(WORKER_ID);
    let client_addr = Address::new_id(CLIENT_ID);
    let control_addr = Address::new_id(CONTROL_ID);

    let provider2 = Address::new_id(501);

    let mut rt = setup();
    rt.set_epoch(current_epoch);

    let [deal1, deal2, deal3]: [DealID; 3] = (end_epoch..end_epoch + 3)
        .map(|epoch| {
            generate_and_publish_deal(
                &mut rt,
                client_addr,
                provider_addr,
                owner_addr,
                worker_addr,
                control_addr,
                start_epoch,
                epoch,
            )
        })
        .collect::<Vec<DealID>>()
        .try_into()
        .unwrap();
    activate_deals(&mut rt, sector_expiry, provider_addr, current_epoch, &[deal1, deal2, deal3]);

    let deal4 = generate_and_publish_deal(
        &mut rt,
        client_addr,
        provider2,
        owner_addr,
        worker_addr,
        control_addr,
        start_epoch,
        end_epoch,
    );
    let deal5 = generate_and_publish_deal(
        &mut rt,
        client_addr,
        provider2,
        owner_addr,
        worker_addr,
        control_addr,
        start_epoch,
        end_epoch + 1,
    );
    activate_deals(&mut rt, sector_expiry, provider2, current_epoch, &[deal4, deal5]);

    terminate_deals(&mut rt, provider_addr, &[deal1]);
    assert_deals_terminated(&mut rt, current_epoch, &[deal1]);
    assert_deals_not_terminated(&mut rt, &[deal2, deal3, deal4, deal5]);

    terminate_deals(&mut rt, provider2, &[deal5]);
    assert_deals_terminated(&mut rt, current_epoch, &[deal5]);
    assert_deals_not_terminated(&mut rt, &[deal2, deal3, deal4]);

    terminate_deals(&mut rt, provider_addr, &[deal2, deal3]);
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
    let owner_addr = Address::new_id(OWNER_ID);
    let provider_addr = Address::new_id(PROVIDER_ID);
    let worker_addr = Address::new_id(WORKER_ID);
    let client_addr = Address::new_id(CLIENT_ID);
    let control_addr = Address::new_id(CONTROL_ID);

    let mut rt = setup();
    rt.set_epoch(current_epoch);

    let deal1 = generate_and_publish_deal(
        &mut rt,
        client_addr,
        provider_addr,
        owner_addr,
        worker_addr,
        control_addr,
        start_epoch,
        end_epoch,
    );
    activate_deals(&mut rt, sector_expiry, provider_addr, current_epoch, &[deal1]);

    terminate_deals(&mut rt, provider_addr, &[deal1, 42]);

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
    let owner_addr = Address::new_id(OWNER_ID);
    let provider_addr = Address::new_id(PROVIDER_ID);
    let worker_addr = Address::new_id(WORKER_ID);
    let client_addr = Address::new_id(CLIENT_ID);
    let control_addr = Address::new_id(CONTROL_ID);

    let mut rt = setup();
    rt.set_epoch(current_epoch);

    let deal1 = generate_and_publish_deal(
        &mut rt,
        client_addr,
        provider_addr,
        owner_addr,
        worker_addr,
        control_addr,
        start_epoch,
        end_epoch,
    );
    let deal2 = generate_and_publish_deal(
        &mut rt,
        client_addr,
        provider_addr,
        owner_addr,
        worker_addr,
        control_addr,
        start_epoch,
        end_epoch + 1,
    );
    let deal3 = generate_and_publish_deal(
        &mut rt,
        client_addr,
        provider_addr,
        owner_addr,
        worker_addr,
        control_addr,
        start_epoch,
        end_epoch - 1,
    );
    activate_deals(&mut rt, sector_expiry, provider_addr, current_epoch, &[deal1, deal2, deal3]);

    let new_epoch = end_epoch - 1;
    rt.set_epoch(new_epoch);

    terminate_deals(&mut rt, provider_addr, &[deal1, deal2, deal3]);
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
    let owner_addr = Address::new_id(OWNER_ID);
    let provider_addr = Address::new_id(PROVIDER_ID);
    let worker_addr = Address::new_id(WORKER_ID);
    let client_addr = Address::new_id(CLIENT_ID);
    let control_addr = Address::new_id(CONTROL_ID);

    let mut rt = setup();
    rt.set_epoch(current_epoch);

    let deal1 = generate_deal_and_add_funds(
        &mut rt,
        client_addr,
        provider_addr,
        owner_addr,
        worker_addr,
        start_epoch,
        end_epoch,
    );
    let deal2 = generate_deal_and_add_funds(
        &mut rt,
        client_addr,
        provider_addr,
        owner_addr,
        worker_addr,
        start_epoch,
        end_epoch - deal_updates_interval,
    );

    rt.set_caller(*ACCOUNT_ACTOR_CODE_ID, worker_addr);
    let deal_ids = publish_deals(
        &mut rt,
        provider_addr,
        owner_addr,
        worker_addr,
        control_addr,
        &[deal1, deal2.clone()],
    );
    activate_deals(&mut rt, sector_expiry, provider_addr, current_epoch, &deal_ids);

    let new_epoch = end_epoch - 1;
    rt.set_epoch(new_epoch);
    cron_tick(&mut rt);

    terminate_deals(&mut rt, provider_addr, &deal_ids);
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
    let owner_addr = Address::new_id(OWNER_ID);
    let provider_addr = Address::new_id(PROVIDER_ID);
    let worker_addr = Address::new_id(WORKER_ID);
    let client_addr = Address::new_id(CLIENT_ID);
    let control_addr = Address::new_id(CONTROL_ID);

    let mut rt = setup();
    rt.set_epoch(current_epoch);

    let deal1 = generate_and_publish_deal(
        &mut rt,
        client_addr,
        provider_addr,
        owner_addr,
        worker_addr,
        control_addr,
        start_epoch,
        end_epoch,
    );
    activate_deals(&mut rt, sector_expiry, provider_addr, current_epoch, &[deal1]);

    // terminating the deal so slash epoch is the current epoch
    terminate_deals(&mut rt, provider_addr, &[deal1]);

    // set a new epoch and terminate again -> however slash epoch will still be the old epoch.
    rt.set_epoch(current_epoch + 1);
    terminate_deals(&mut rt, provider_addr, &[deal1]);
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
    let owner_addr = Address::new_id(OWNER_ID);
    let provider_addr = Address::new_id(PROVIDER_ID);
    let worker_addr = Address::new_id(WORKER_ID);
    let client_addr = Address::new_id(CLIENT_ID);
    let control_addr = Address::new_id(CONTROL_ID);

    let mut rt = setup();
    rt.set_epoch(current_epoch);

    // provider1 publishes deal1 and 2 and deal3 -> deal3 has the lowest endepoch
    let deals: Vec<DealID> = [end_epoch, end_epoch + 1, end_epoch - 1]
        .iter()
        .map(|&epoch| {
            generate_and_publish_deal(
                &mut rt,
                client_addr,
                provider_addr,
                owner_addr,
                worker_addr,
                control_addr,
                start_epoch,
                epoch,
            )
        })
        .collect();
    let [deal1, deal2, deal3]: [DealID; 3] = deals.as_slice().try_into().unwrap();
    activate_deals(&mut rt, sector_expiry, provider_addr, current_epoch, &deals);

    // terminating the deal so slash epoch is the current epoch
    terminate_deals(&mut rt, provider_addr, &[deal1]);

    // set a new epoch and terminate again -> however slash epoch will still be the old epoch.
    let new_epoch = current_epoch + 1;
    rt.set_epoch(new_epoch);
    terminate_deals(&mut rt, provider_addr, &deals);

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
    let owner_addr = Address::new_id(OWNER_ID);
    let provider_addr = Address::new_id(PROVIDER_ID);
    let worker_addr = Address::new_id(WORKER_ID);
    let client_addr = Address::new_id(CLIENT_ID);
    let control_addr = Address::new_id(CONTROL_ID);

    let mut rt = setup();
    rt.set_epoch(current_epoch);

    // deal1 has endepoch equal to current epoch when terminate is called
    let deal1 = generate_and_publish_deal(
        &mut rt,
        client_addr,
        provider_addr,
        owner_addr,
        worker_addr,
        control_addr,
        start_epoch,
        end_epoch,
    );
    activate_deals(&mut rt, sector_expiry, provider_addr, current_epoch, &[deal1]);
    rt.set_epoch(end_epoch);
    terminate_deals(&mut rt, provider_addr, &[deal1]);
    assert_deals_not_terminated(&mut rt, &[deal1]);

    // deal2 has end epoch less than current epoch when terminate is called
    rt.set_epoch(current_epoch);
    let deal2 = generate_and_publish_deal(
        &mut rt,
        client_addr,
        provider_addr,
        owner_addr,
        worker_addr,
        control_addr,
        start_epoch + 1,
        end_epoch,
    );
    activate_deals(&mut rt, sector_expiry, provider_addr, current_epoch, &[deal2]);
    rt.set_epoch(end_epoch + 1);
    terminate_deals(&mut rt, provider_addr, &[deal2]);
    assert_deals_not_terminated(&mut rt, &[deal2]);
}

// Converted from: https://github.com/filecoin-project/specs-actors/blob/master/actors/builtin/market/market_test.go#L1436
#[test]
fn fail_when_caller_is_not_a_storage_miner_actor() {
    let provider_addr = Address::new_id(PROVIDER_ID);

    let mut rt = setup();
    rt.expect_validate_caller_type(vec![*MINER_ACTOR_CODE_ID]);
    rt.set_caller(*ACCOUNT_ACTOR_CODE_ID, provider_addr);
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
    let owner_addr = Address::new_id(OWNER_ID);
    let provider_addr = Address::new_id(PROVIDER_ID);
    let worker_addr = Address::new_id(WORKER_ID);
    let client_addr = Address::new_id(CLIENT_ID);
    let control_addr = Address::new_id(CONTROL_ID);

    let provider2 = Address::new_id(501);

    let mut rt = setup();
    rt.set_epoch(current_epoch);

    let deal = generate_and_publish_deal(
        &mut rt,
        client_addr,
        provider_addr,
        owner_addr,
        worker_addr,
        control_addr,
        start_epoch,
        end_epoch,
    );
    activate_deals(&mut rt, sector_expiry, provider_addr, current_epoch, &[deal]);

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
    let owner_addr = Address::new_id(OWNER_ID);
    let provider_addr = Address::new_id(PROVIDER_ID);
    let worker_addr = Address::new_id(WORKER_ID);
    let client_addr = Address::new_id(CLIENT_ID);
    let control_addr = Address::new_id(CONTROL_ID);

    let mut rt = setup();
    rt.set_epoch(current_epoch);

    let deal = generate_and_publish_deal(
        &mut rt,
        client_addr,
        provider_addr,
        owner_addr,
        worker_addr,
        control_addr,
        start_epoch,
        end_epoch,
    );

    let ret = terminate_deals_raw(&mut rt, provider_addr, &[deal]);
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
    let owner_addr = Address::new_id(OWNER_ID);
    let provider_addr = Address::new_id(PROVIDER_ID);
    let worker_addr = Address::new_id(WORKER_ID);
    let client_addr = Address::new_id(CLIENT_ID);
    let control_addr = Address::new_id(CONTROL_ID);

    let mut rt = setup();
    rt.set_epoch(current_epoch);

    // deal1 would terminate but deal2 will fail because deal2 has not been activated
    let deal1 = generate_and_publish_deal(
        &mut rt,
        client_addr,
        provider_addr,
        owner_addr,
        worker_addr,
        control_addr,
        start_epoch,
        end_epoch,
    );
    activate_deals(&mut rt, sector_expiry, provider_addr, current_epoch, &[deal1]);
    let deal2 = generate_and_publish_deal(
        &mut rt,
        client_addr,
        provider_addr,
        owner_addr,
        worker_addr,
        control_addr,
        start_epoch,
        end_epoch + 1,
    );

    let ret = terminate_deals_raw(&mut rt, provider_addr, &[deal1, deal2]);
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

    let owner_addr = Address::new_id(OWNER_ID);
    let provider_addr = Address::new_id(PROVIDER_ID);
    let worker_addr = Address::new_id(WORKER_ID);
    let client_addr = Address::new_id(CLIENT_ID);
    let control_addr = Address::new_id(CONTROL_ID);

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
        client_addr,
        provider_addr,
        owner_addr,
        worker_addr,
        provider_collateral,
        client_collateral,
        start_epoch,
        end_epoch,
    );
    let qa_power = StoragePower::from_i128(1 << 50).unwrap();
    rt.set_circulating_supply(qa_power); // convenient for these two numbers to cancel out

    // publish the deal successfully
    rt.set_epoch(publish_epoch);
    rt.set_caller(*ACCOUNT_ACTOR_CODE_ID, worker_addr);
    publish_deals(&mut rt, provider_addr, owner_addr, worker_addr, control_addr, &[deal]);
    // TODO: actor.checkState(rt)
}

#[test]
fn publish_multiple_deals_for_different_clients_and_ensure_balances_are_correct() {
    let start_epoch = 42;
    let end_epoch = start_epoch + 200 * EPOCHS_IN_DAY;

    let mut rt = setup();

    let client1_addr = Address::new_id(900);
    let client2_addr = Address::new_id(901);
    let client3_addr = Address::new_id(902);

    let owner_addr = Address::new_id(OWNER_ID);
    let provider_addr = Address::new_id(PROVIDER_ID);
    let worker_addr = Address::new_id(WORKER_ID);
    let control_addr = Address::new_id(CONTROL_ID);

    // generate first deal for
    let deal1 = generate_deal_and_add_funds(
        &mut rt,
        client1_addr,
        provider_addr,
        owner_addr,
        worker_addr,
        start_epoch,
        end_epoch,
    );

    // generate second deal
    let deal2 = generate_deal_and_add_funds(
        &mut rt,
        client2_addr,
        provider_addr,
        owner_addr,
        worker_addr,
        start_epoch,
        end_epoch,
    );

    // generate third deal
    let deal3 = generate_deal_and_add_funds(
        &mut rt,
        client3_addr,
        provider_addr,
        owner_addr,
        worker_addr,
        start_epoch,
        end_epoch,
    );

    rt.set_caller(*ACCOUNT_ACTOR_CODE_ID, worker_addr);
    publish_deals(
        &mut rt,
        provider_addr,
        owner_addr,
        worker_addr,
        control_addr,
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
    assert_eq!(provider_locked_expected, get_locked_balance(&mut rt, provider_addr));

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
        provider_addr,
        owner_addr,
        worker_addr,
        1000,
        1000 + 200 * EPOCHS_IN_DAY,
    );
    let deal5 = generate_deal_and_add_funds(
        &mut rt,
        client3_addr,
        provider_addr,
        owner_addr,
        worker_addr,
        100,
        100 + 200 * EPOCHS_IN_DAY,
    );
    rt.set_caller(*ACCOUNT_ACTOR_CODE_ID, worker_addr);
    publish_deals(
        &mut rt,
        provider_addr,
        owner_addr,
        worker_addr,
        control_addr,
        &[deal4.clone(), deal5.clone()],
    );

    // assert locked balances for clients and provider
    let provider_locked_expected =
        &provider_locked_expected + &deal4.provider_collateral + &deal5.provider_collateral;
    assert_eq!(provider_locked_expected, get_locked_balance(&mut rt, provider_addr));

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
    let deal6 = generate_deal_and_add_funds(
        &mut rt,
        client1_addr,
        provider2_addr,
        owner_addr,
        worker_addr,
        20,
        20 + 200 * EPOCHS_IN_DAY,
    );

    // generate second deal for second provider
    let deal7 = generate_deal_and_add_funds(
        &mut rt,
        client1_addr,
        provider2_addr,
        owner_addr,
        worker_addr,
        25,
        60 + 200 * EPOCHS_IN_DAY,
    );

    // publish both the deals for the second provider
    rt.set_caller(*ACCOUNT_ACTOR_CODE_ID, worker_addr);
    publish_deals(
        &mut rt,
        provider2_addr,
        owner_addr,
        worker_addr,
        control_addr,
        &[deal6.clone(), deal7.clone()],
    );

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
    assert_eq!(provider_locked_expected, get_locked_balance(&mut rt, provider_addr));

    let total_client_collateral_locked =
        &total_client_collateral_locked + &deal6.client_collateral + &deal7.client_collateral;
    assert_eq!(total_client_collateral_locked, st.total_client_locked_collateral);
    assert_eq!(provider_locked_expected + provider2_locked, st.total_provider_locked_collateral);
    let total_storage_fee =
        &total_storage_fee + &deal6.total_storage_fee() + &deal7.total_storage_fee();
    assert_eq!(total_storage_fee, st.total_client_storage_fee);
    // TODO: actor.checkState(rt)
}

#[test]
fn active_deals_multiple_times_with_different_providers() {
    let start_epoch = 10;
    let end_epoch = start_epoch + 200 * EPOCHS_IN_DAY;
    let current_epoch = ChainEpoch::from(5);
    let sector_expiry = end_epoch + 100;

    let mut rt = setup();
    rt.set_epoch(current_epoch);

    let owner_addr = Address::new_id(OWNER_ID);
    let provider_addr = Address::new_id(PROVIDER_ID);
    let worker_addr = Address::new_id(WORKER_ID);
    let client_addr = Address::new_id(CLIENT_ID);
    let control_addr = Address::new_id(CONTROL_ID);

    // provider 1 publishes deals1 and deals2 and deal3
    let deal1 = generate_and_publish_deal(
        &mut rt,
        client_addr,
        provider_addr,
        owner_addr,
        worker_addr,
        control_addr,
        start_epoch,
        end_epoch,
    );
    let deal2 = generate_and_publish_deal(
        &mut rt,
        client_addr,
        provider_addr,
        owner_addr,
        worker_addr,
        control_addr,
        start_epoch,
        end_epoch + 1,
    );
    let deal3 = generate_and_publish_deal(
        &mut rt,
        client_addr,
        provider_addr,
        owner_addr,
        worker_addr,
        control_addr,
        start_epoch,
        end_epoch + 2,
    );

    // provider2 publishes deal4 and deal5
    let provider2_addr = Address::new_id(401);
    let deal4 = generate_and_publish_deal(
        &mut rt,
        client_addr,
        provider2_addr,
        owner_addr,
        worker_addr,
        control_addr,
        start_epoch,
        end_epoch,
    );
    let deal5 = generate_and_publish_deal(
        &mut rt,
        client_addr,
        provider2_addr,
        owner_addr,
        worker_addr,
        control_addr,
        start_epoch,
        end_epoch + 1,
    );

    // provider1 activates deal1 and deal2 but that does not activate deal3 to deal5
    activate_deals(&mut rt, sector_expiry, provider_addr, current_epoch, &[deal1, deal2]);
    assert_deals_not_activated(&mut rt, current_epoch, &[deal3, deal4, deal5]);

    // provider2 activates deal5 but that does not activate deal3 or deal4
    activate_deals(&mut rt, sector_expiry, provider2_addr, current_epoch, &[deal5]);
    assert_deals_not_activated(&mut rt, current_epoch, &[deal3, deal4]);

    // provider1 activates deal3
    activate_deals(&mut rt, sector_expiry, provider_addr, current_epoch, &[deal3]);
    assert_deals_not_activated(&mut rt, current_epoch, &[deal4]);
    // TODO: actor.checkState(rt)
}

// Converted from: https://github.com/filecoin-project/specs-actors/blob/master/actors/builtin/market/market_test.go#L1519
#[test]
fn fail_when_deal_is_activated_but_proposal_is_not_found() {
    let start_epoch = 50;
    let end_epoch = start_epoch + 200 * EPOCHS_IN_DAY;
    let sector_expiry = end_epoch + 100;

    let mut rt = setup();

    let owner_addr = Address::new_id(OWNER_ID);
    let provider_addr = Address::new_id(PROVIDER_ID);
    let worker_addr = Address::new_id(WORKER_ID);
    let client_addr = Address::new_id(CLIENT_ID);
    let control_addr = Address::new_id(CONTROL_ID);

    let deal_id = publish_and_activate_deal(
        &mut rt,
        client_addr,
        provider_addr,
        owner_addr,
        worker_addr,
        control_addr,
        start_epoch,
        end_epoch,
        0,
        sector_expiry,
    );

    // delete the deal proposal (this breaks state invariants)
    delete_deal_proposal(&mut rt, deal_id);

    rt.set_epoch(process_epoch(start_epoch, deal_id));
    expect_abort(ExitCode::USR_NOT_FOUND, cron_tick_raw(&mut rt));

    // TODO: actor.checkState
}

// Converted from: https://github.com/filecoin-project/specs-actors/blob/master/actors/builtin/market/market_test.go#L1540
#[test]
fn fail_when_deal_update_epoch_is_in_the_future() {
    let start_epoch = 50;
    let end_epoch = start_epoch + 200 * EPOCHS_IN_DAY;
    let sector_expiry = end_epoch + 100;

    let mut rt = setup();

    let owner_addr = Address::new_id(OWNER_ID);
    let provider_addr = Address::new_id(PROVIDER_ID);
    let worker_addr = Address::new_id(WORKER_ID);
    let client_addr = Address::new_id(CLIENT_ID);
    let control_addr = Address::new_id(CONTROL_ID);

    let deal_id = publish_and_activate_deal(
        &mut rt,
        client_addr,
        provider_addr,
        owner_addr,
        worker_addr,
        control_addr,
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

    // TODO: actor.checkState
}

#[test]
fn crontick_for_a_deal_at_its_start_epoch_results_in_zero_payment_and_no_slashing() {
    let start_epoch = ChainEpoch::from(50);
    let end_epoch = start_epoch + 200 * EPOCHS_IN_DAY;
    let sector_expiry = end_epoch + 100;

    let client_addr = Address::new_id(CLIENT_ID);
    let provider_addr = Address::new_id(PROVIDER_ID);
    let owner_addr = Address::new_id(OWNER_ID);
    let worker_addr = Address::new_id(WORKER_ID);
    let control_addr = Address::new_id(CONTROL_ID);

    // set start epoch to coincide with processing (0 + 0 % 2880 = 0)
    let start_epoch = 0;
    let mut rt = setup();
    let deal_id = publish_and_activate_deal(
        &mut rt,
        client_addr,
        provider_addr,
        owner_addr,
        worker_addr,
        control_addr,
        start_epoch,
        end_epoch,
        0,
        sector_expiry,
    );

    // move the current epoch to processing epoch
    let current = process_epoch(start_epoch, deal_id);
    rt.set_epoch(current);
    let (pay, slashed) =
        cron_tick_and_assert_balances(&mut rt, client_addr, provider_addr, current, deal_id);
    assert_eq!(TokenAmount::from(0u8), pay);
    assert_eq!(TokenAmount::from(0u8), slashed);

    // deal proposal and state should NOT be deleted
    get_deal_proposal(&mut rt, deal_id);
    get_deal_state(&mut rt, deal_id);
    // TODO: actor.checkState(rt)
}

#[test]
fn slash_a_deal_and_make_payment_for_another_deal_in_the_same_epoch() {
    let start_epoch = ChainEpoch::from(50);
    let end_epoch = start_epoch + 200 * EPOCHS_IN_DAY;
    let sector_expiry = end_epoch + 100;

    let client_addr = Address::new_id(CLIENT_ID);
    let provider_addr = Address::new_id(PROVIDER_ID);
    let owner_addr = Address::new_id(OWNER_ID);
    let worker_addr = Address::new_id(WORKER_ID);
    let control_addr = Address::new_id(CONTROL_ID);

    let mut rt = setup();

    let deal_id1 = publish_and_activate_deal(
        &mut rt,
        client_addr,
        provider_addr,
        owner_addr,
        worker_addr,
        control_addr,
        start_epoch,
        end_epoch,
        0,
        sector_expiry,
    );
    let d1 = get_deal_proposal(&mut rt, deal_id1);

    let deal_id2 = publish_and_activate_deal(
        &mut rt,
        client_addr,
        provider_addr,
        owner_addr,
        worker_addr,
        control_addr,
        start_epoch + 1,
        end_epoch + 1,
        0,
        sector_expiry,
    );

    // slash deal1
    let slash_epoch = process_epoch(start_epoch, deal_id2) + ChainEpoch::from(100);
    rt.set_epoch(slash_epoch);
    terminate_deals(&mut rt, provider_addr, &[deal_id1]);

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
    // TODO: actor.checkState(rt)
}

#[test]
fn cannot_publish_the_same_deal_twice_before_a_cron_tick() {
    let start_epoch = ChainEpoch::from(50);
    let end_epoch = start_epoch + 200 * EPOCHS_IN_DAY;

    let client_addr = Address::new_id(CLIENT_ID);
    let provider_addr = Address::new_id(PROVIDER_ID);
    let owner_addr = Address::new_id(OWNER_ID);
    let worker_addr = Address::new_id(WORKER_ID);
    let control_addr = Address::new_id(CONTROL_ID);

    // Publish a deal
    let mut rt = setup();
    generate_and_publish_deal(
        &mut rt,
        client_addr,
        provider_addr,
        owner_addr,
        worker_addr,
        control_addr,
        start_epoch,
        end_epoch,
    );

    // now try to publish it again and it should fail because it will still be in pending state
    let d2 = generate_deal_and_add_funds(
        &mut rt,
        client_addr,
        provider_addr,
        owner_addr,
        worker_addr,
        start_epoch,
        end_epoch,
    );
    let buf = RawBytes::serialize(d2.clone()).expect("failed to marshal deal proposal");
    let sig = Signature::new_bls("does not matter".as_bytes().to_vec());
    let params = PublishStorageDealsParams {
        deals: vec![ClientDealProposal { proposal: d2.clone(), client_signature: sig.clone() }],
    };
    rt.expect_validate_caller_type(vec![*ACCOUNT_ACTOR_CODE_ID, *MULTISIG_ACTOR_CODE_ID]);
    expect_provider_control_address(&mut rt, provider_addr, owner_addr, worker_addr);
    expect_query_network_info(&mut rt);
    rt.set_caller(*ACCOUNT_ACTOR_CODE_ID, worker_addr);
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
    let client_addr = Address::new_id(CLIENT_ID);
    let provider_addr = Address::new_id(PROVIDER_ID);
    let owner_addr = Address::new_id(OWNER_ID);
    let worker_addr = Address::new_id(WORKER_ID);
    let control_addr = Address::new_id(CONTROL_ID);

    let start_epoch = 10;
    let end_epoch = start_epoch + 200 * EPOCHS_IN_DAY;
    let sector_expiry = end_epoch + 100;

    let mut rt = setup();
    let deal_id = generate_and_publish_deal(
        &mut rt,
        client_addr,
        provider_addr,
        owner_addr,
        worker_addr,
        control_addr,
        start_epoch,
        end_epoch,
    );

    rt.expect_validate_caller_type(vec![*MINER_ACTOR_CODE_ID]);
    rt.set_caller(*MINER_ACTOR_CODE_ID, provider_addr);
    rt.set_epoch(start_epoch + 1);
    let params = ActivateDealsParams { deal_ids: vec![deal_id], sector_expiry };
    expect_abort(
        ExitCode::USR_ILLEGAL_ARGUMENT,
        rt.call::<MarketActor>(Method::ActivateDeals as u64, &RawBytes::serialize(params).unwrap()),
    );

    rt.verify();
    // TODO: actor.checkState(rt)
}

#[test]
fn fail_when_end_epoch_of_deal_greater_than_sector_expiry() {
    let client_addr = Address::new_id(CLIENT_ID);
    let provider_addr = Address::new_id(PROVIDER_ID);
    let owner_addr = Address::new_id(OWNER_ID);
    let worker_addr = Address::new_id(WORKER_ID);
    let control_addr = Address::new_id(CONTROL_ID);

    let start_epoch = 10;
    let end_epoch = start_epoch + 200 * EPOCHS_IN_DAY;

    let mut rt = setup();
    let deal_id = generate_and_publish_deal(
        &mut rt,
        client_addr,
        provider_addr,
        owner_addr,
        worker_addr,
        control_addr,
        start_epoch,
        end_epoch,
    );

    rt.expect_validate_caller_type(vec![*MINER_ACTOR_CODE_ID]);
    rt.set_caller(*MINER_ACTOR_CODE_ID, provider_addr);
    let params = ActivateDealsParams { deal_ids: vec![deal_id], sector_expiry: end_epoch - 1 };
    expect_abort(
        ExitCode::USR_ILLEGAL_ARGUMENT,
        rt.call::<MarketActor>(Method::ActivateDeals as u64, &RawBytes::serialize(params).unwrap()),
    );

    rt.verify();
    // TODO: actor.checkState(rt)
}

#[test]
fn fail_to_activate_all_deals_if_one_deal_fails() {
    let client_addr = Address::new_id(CLIENT_ID);
    let provider_addr = Address::new_id(PROVIDER_ID);
    let owner_addr = Address::new_id(OWNER_ID);
    let worker_addr = Address::new_id(WORKER_ID);
    let control_addr = Address::new_id(CONTROL_ID);

    let start_epoch = 10;
    let end_epoch = start_epoch + 200 * EPOCHS_IN_DAY;
    let sector_expiry = end_epoch + 100;

    let mut rt = setup();
    // activate deal1 so it fails later
    let deal_id1 = generate_and_publish_deal(
        &mut rt,
        client_addr,
        provider_addr,
        owner_addr,
        worker_addr,
        control_addr,
        start_epoch,
        end_epoch,
    );
    activate_deals(&mut rt, sector_expiry, provider_addr, 0, &[deal_id1]);

    let deal_id2 = generate_and_publish_deal(
        &mut rt,
        client_addr,
        provider_addr,
        owner_addr,
        worker_addr,
        control_addr,
        start_epoch,
        end_epoch + 1,
    );

    rt.expect_validate_caller_type(vec![*MINER_ACTOR_CODE_ID]);
    rt.set_caller(*MINER_ACTOR_CODE_ID, provider_addr);
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
    // TODO: actor.checkState(rt)
}

fn expect_provider_control_address(
    rt: &mut MockRuntime,
    provider: Address,
    owner: Address,
    worker: Address,
) {
    expect_get_control_addresses(rt, provider, owner, worker, vec![])
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

fn activate_deals(
    rt: &mut MockRuntime,
    sector_expiry: ChainEpoch,
    provider: Address,
    current_epoch: ChainEpoch,
    deal_ids: &[DealID],
) {
    rt.set_caller(*MINER_ACTOR_CODE_ID, provider);
    rt.expect_validate_caller_type(vec![*MINER_ACTOR_CODE_ID]);

    let params = ActivateDealsParams { deal_ids: deal_ids.to_vec(), sector_expiry };

    let ret = rt
        .call::<MarketActor>(Method::ActivateDeals as u64, &RawBytes::serialize(params).unwrap())
        .unwrap();
    assert_eq!(ret, RawBytes::default());
    rt.verify();

    for d in deal_ids {
        let s = get_deal_state(rt, *d);
        assert_eq!(current_epoch, s.sector_start_epoch);
    }
}

fn get_deal_proposal(rt: &mut MockRuntime, deal_id: DealID) -> DealProposal {
    let st: State = rt.get_state();

    let deals = DealArray::load(&st.proposals, &rt.store).unwrap();

    let d = deals.get(deal_id).unwrap();
    d.unwrap().clone()
}

fn get_locked_balance(rt: &mut MockRuntime, addr: Address) -> TokenAmount {
    let st: State = rt.get_state();

    let lt = BalanceTable::from_root(&rt.store, &st.locked_table).unwrap();

    lt.get(&addr).unwrap()
}

fn get_deal_state(rt: &mut MockRuntime, deal_id: DealID) -> DealState {
    let st: State = rt.get_state();

    let states = DealMetaArray::load(&st.states, &rt.store).unwrap();

    let s = states.get(deal_id).unwrap();
    *s.unwrap()
}

fn update_last_updated(rt: &mut MockRuntime, deal_id: DealID, new_last_updated: ChainEpoch) {
    let st: State = rt.get_state();

    let mut states = DealMetaArray::load(&st.states, &rt.store).unwrap();
    let s = *states.get(deal_id).unwrap().unwrap();

    states.set(deal_id, DealState { last_updated_epoch: new_last_updated, ..s }).unwrap();
    let root = states.flush().unwrap();
    rt.replace_state(&State { states: root, ..st })
}

fn delete_deal_proposal(rt: &mut MockRuntime, deal_id: DealID) {
    let mut st: State = rt.get_state();

    let mut deals = DealArray::load(&st.proposals, &rt.store).unwrap();
    deals.delete(deal_id).unwrap();

    let root = deals.flush().unwrap();
    st.proposals = root;
    rt.replace_state(&st)
}

#[allow(clippy::too_many_arguments)]
fn generate_and_publish_deal(
    rt: &mut MockRuntime,
    client: Address,
    provider: Address,
    owner: Address,
    worker: Address,
    control: Address,
    start_epoch: ChainEpoch,
    end_epoch: ChainEpoch,
) -> DealID {
    let deal =
        generate_deal_and_add_funds(rt, client, provider, owner, worker, start_epoch, end_epoch);
    rt.set_caller(*ACCOUNT_ACTOR_CODE_ID, worker);
    let deal_ids = publish_deals(rt, provider, owner, worker, control, &[deal]);
    deal_ids[0]
}

#[allow(clippy::too_many_arguments)]
fn generate_and_publish_deal_for_piece(
    rt: &mut MockRuntime,
    client: Address,
    provider: Address,
    owner: Address,
    worker: Address,
    control: Address,
    start_epoch: ChainEpoch,
    end_epoch: ChainEpoch,
    piece_cid: Cid,
    piece_size: PaddedPieceSize,
) -> DealID {
    // generate deal
    let storage_per_epoch = BigInt::from(10u8);
    let client_collateral = TokenAmount::from(10u8);
    let provider_collateral = TokenAmount::from(10u8);

    let deal = DealProposal {
        piece_cid,
        piece_size,
        verified_deal: true,
        client,
        provider,
        label: "label".to_string(),
        start_epoch,
        end_epoch,
        storage_price_per_epoch: storage_per_epoch,
        provider_collateral,
        client_collateral,
    };

    // add funds
    add_provider_funds(rt, deal.provider_collateral.clone(), provider, owner, worker);
    add_participant_funds(rt, client, deal.client_balance_requirement());

    // publish
    rt.set_caller(*ACCOUNT_ACTOR_CODE_ID, worker);
    let deal_ids = publish_deals(rt, provider, owner, worker, control, &[deal]);
    deal_ids[0]
}

fn generate_deal_and_add_funds(
    rt: &mut MockRuntime,
    client: Address,
    provider: Address,
    owner: Address,
    worker: Address,
    start_epoch: ChainEpoch,
    end_epoch: ChainEpoch,
) -> DealProposal {
    let deal = generate_deal_proposal(client, provider, start_epoch, end_epoch);
    add_provider_funds(rt, deal.provider_collateral.clone(), provider, owner, worker);
    add_participant_funds(rt, client, deal.client_balance_requirement());
    deal
}

#[allow(clippy::too_many_arguments)]
fn generate_deal_with_collateral_and_add_funds(
    rt: &mut MockRuntime,
    client: Address,
    provider: Address,
    owner: Address,
    worker: Address,
    provider_collateral: BigInt,
    client_collateral: BigInt,
    start_epoch: ChainEpoch,
    end_epoch: ChainEpoch,
) -> DealProposal {
    let deal = generate_deal_proposal_with_collateral(
        client,
        provider,
        client_collateral,
        provider_collateral,
        start_epoch,
        end_epoch,
    );
    add_provider_funds(rt, deal.provider_collateral.clone(), provider, owner, worker);
    add_participant_funds(rt, client, deal.client_balance_requirement());
    deal
}

fn generate_deal_proposal_with_collateral(
    client: Address,
    provider: Address,
    client_collateral: TokenAmount,
    provider_collateral: TokenAmount,
    start_epoch: ChainEpoch,
    end_epoch: ChainEpoch,
) -> DealProposal {
    let piece_cid = make_piece_cid("1".as_bytes());
    let piece_size = PaddedPieceSize(2048u64);
    let storage_per_epoch = BigInt::from(10u8);
    DealProposal {
        piece_cid,
        piece_size,
        verified_deal: true,
        client,
        provider,
        label: "label".to_string(),
        start_epoch,
        end_epoch,
        storage_price_per_epoch: storage_per_epoch,
        provider_collateral,
        client_collateral,
    }
}

fn generate_deal_proposal(
    client: Address,
    provider: Address,
    start_epoch: ChainEpoch,
    end_epoch: ChainEpoch,
) -> DealProposal {
    let client_collateral = TokenAmount::from(10u8);
    let provider_collateral = TokenAmount::from(10u8);
    generate_deal_proposal_with_collateral(
        client,
        provider,
        client_collateral,
        provider_collateral,
        start_epoch,
        end_epoch,
    )
}

fn terminate_deals(rt: &mut MockRuntime, miner_addr: Address, deal_ids: &[DealID]) {
    let ret = terminate_deals_raw(rt, miner_addr, deal_ids).unwrap();
    assert_eq!(ret, RawBytes::default());
    rt.verify();
}

fn terminate_deals_raw(
    rt: &mut MockRuntime,
    miner_addr: Address,
    deal_ids: &[DealID],
) -> Result<RawBytes, ActorError> {
    rt.set_caller(*MINER_ACTOR_CODE_ID, miner_addr);
    rt.expect_validate_caller_type(vec![*MINER_ACTOR_CODE_ID]);

    let params = OnMinerSectorsTerminateParams { epoch: rt.epoch, deal_ids: deal_ids.to_vec() };

    rt.call::<MarketActor>(
        Method::OnMinerSectorsTerminate as u64,
        &RawBytes::serialize(params).unwrap(),
    )
}

#[allow(clippy::too_many_arguments)]
fn publish_and_activate_deal(
    rt: &mut MockRuntime,
    client: Address,
    provider: Address,
    owner: Address,
    worker: Address,
    control: Address,
    start_epoch: ChainEpoch,
    end_epoch: ChainEpoch,
    current_epoch: ChainEpoch,
    sector_expiry: ChainEpoch,
) -> DealID {
    let deal =
        generate_deal_and_add_funds(rt, client, provider, owner, worker, start_epoch, end_epoch);
    rt.set_caller(*ACCOUNT_ACTOR_CODE_ID, worker);
    let deal_ids = publish_deals(rt, provider, owner, worker, control, &[deal]);
    activate_deals(rt, sector_expiry, provider, current_epoch, &deal_ids);
    deal_ids[0]
}

// if this is the first crontick for the deal, it's next tick will be scheduled at `desiredNextEpoch`
// if this is not the first crontick, the `desiredNextEpoch` param is ignored.
fn cron_tick_and_assert_balances(
    rt: &mut MockRuntime,
    client_addr: Address,
    provider_addr: Address,
    current_epoch: ChainEpoch,
    deal_id: DealID,
) -> (TokenAmount, TokenAmount) {
    // fetch current client and provider escrow balances
    let c_locked = get_locked_balance(rt, client_addr);
    let c_escrow = get_escrow_balance(rt, &client_addr).unwrap();
    let p_locked = get_locked_balance(rt, provider_addr);
    let p_escrow = get_escrow_balance(rt, &provider_addr).unwrap();
    let mut amount_slashed = TokenAmount::from(0u8);

    let s = get_deal_state(rt, deal_id);
    let d = get_deal_proposal(rt, deal_id);

    // end epoch for payment calc
    let mut payment_end = d.end_epoch;
    if s.slash_epoch != EPOCH_UNDEFINED {
        rt.expect_send(
            *BURNT_FUNDS_ACTOR_ADDR,
            METHOD_SEND,
            RawBytes::default(),
            d.provider_collateral.clone(),
            RawBytes::default(),
            ExitCode::OK,
        );
        amount_slashed = d.provider_collateral;

        if s.slash_epoch < d.start_epoch {
            payment_end = d.start_epoch;
        } else {
            payment_end = s.slash_epoch;
        }
    } else if current_epoch < payment_end {
        payment_end = current_epoch;
    }

    // start epoch for payment calc
    let mut payment_start = d.start_epoch;
    if s.last_updated_epoch != EPOCH_UNDEFINED {
        payment_start = s.last_updated_epoch;
    }
    let duration = payment_end - payment_start;
    let payment = duration * d.storage_price_per_epoch;

    // expected updated amounts
    let updated_client_escrow = c_escrow - payment.clone();
    let updated_provider_escrow = (p_escrow + payment.clone()) - amount_slashed.clone();
    let mut updated_client_locked = c_locked - payment.clone();
    let mut updated_provider_locked = p_locked;
    // if the deal has expired or been slashed, locked amount will be zero for provider and client.
    let is_deal_expired = payment_end == d.end_epoch;
    if is_deal_expired || s.slash_epoch != EPOCH_UNDEFINED {
        updated_client_locked = TokenAmount::from(0u8);
        updated_provider_locked = TokenAmount::from(0u8);
    }

    cron_tick(rt);

    assert_eq!(updated_client_escrow, get_escrow_balance(rt, &client_addr).unwrap());
    assert_eq!(updated_client_locked, get_locked_balance(rt, client_addr));
    assert_eq!(updated_provider_escrow, get_escrow_balance(rt, &provider_addr).unwrap());
    assert_eq!(updated_provider_locked, get_locked_balance(rt, provider_addr));
    (payment, amount_slashed)
}

fn publish_deals(
    rt: &mut MockRuntime,
    provider: Address,
    owner: Address,
    worker: Address,
    control: Address,
    publish_deals: &[DealProposal],
) -> Vec<DealID> {
    rt.expect_validate_caller_type((*CALLER_TYPES_SIGNABLE).clone());

    let return_value = ext::miner::GetControlAddressesReturnParams {
        owner,
        worker,
        control_addresses: vec![control],
    };
    rt.expect_send(
        provider,
        ext::miner::CONTROL_ADDRESSES_METHOD,
        RawBytes::default(),
        TokenAmount::from(0u8),
        RawBytes::serialize(return_value).unwrap(),
        ExitCode::OK,
    );

    expect_query_network_info(rt);

    let mut params: PublishStorageDealsParams = PublishStorageDealsParams { deals: vec![] };

    for deal in publish_deals {
        // create a client proposal with a valid signature
        let buf = RawBytes::serialize(deal.clone()).expect("failed to marshal deal proposal");
        let sig = Signature::new_bls("does not matter".as_bytes().to_vec());
        let client_proposal =
            ClientDealProposal { proposal: deal.clone(), client_signature: sig.clone() };
        params.deals.push(client_proposal);

        // expect a call to verify the above signature
        rt.expect_verify_signature(ExpectedVerifySig {
            sig,
            signer: deal.client,
            plaintext: buf.to_vec(),
            result: Ok(()),
        });
        if deal.verified_deal {
            let param = RawBytes::serialize(UseBytesParams {
                address: deal.client,
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
        }
    }

    let ret: PublishStorageDealsReturn = rt
        .call::<MarketActor>(
            Method::PublishStorageDeals as u64,
            &RawBytes::serialize(params).unwrap(),
        )
        .unwrap()
        .deserialize()
        .unwrap();
    rt.verify();

    assert_eq!(ret.ids.len(), publish_deals.len());

    // assert state after publishing the deals
    for (i, deal_id) in ret.ids.iter().enumerate() {
        let expected = &publish_deals[i];
        let p = get_deal_proposal(rt, *deal_id);

        assert_eq!(expected, &p);
    }

    ret.ids
}

fn assert_deals_not_activated(rt: &mut MockRuntime, _epoch: ChainEpoch, deal_ids: &[DealID]) {
    let st: State = rt.get_state();

    let states = DealMetaArray::load(&st.states, &rt.store).unwrap();

    for d in deal_ids {
        let opt = states.get(*d).unwrap();
        assert!(opt.is_none());
    }
}

fn cron_tick(rt: &mut MockRuntime) {
    assert_eq!(RawBytes::default(), cron_tick_raw(rt).unwrap());
    rt.verify()
}

fn cron_tick_raw(rt: &mut MockRuntime) -> Result<RawBytes, ActorError> {
    rt.expect_validate_caller_addr(vec![*CRON_ACTOR_ADDR]);
    rt.set_caller(*CRON_ACTOR_CODE_ID, *CRON_ACTOR_ADDR);

    rt.call::<MarketActor>(Method::CronTick as u64, &RawBytes::default())
}

fn expect_query_network_info(rt: &mut MockRuntime) {
    //networkQAPower
    //networkBaselinePower
    let rwd = TokenAmount::from(10u8) * TokenAmount::from(10_i128.pow(18));
    let power = StoragePower::from_i128(1 << 50).unwrap();
    let epoch_reward_smooth = FilterEstimate::new(rwd.clone(), BigInt::from(0u8));

    let current_power = CurrentTotalPowerReturn {
        raw_byte_power: StoragePower::default(),
        quality_adj_power: power.clone(),
        pledge_collateral: TokenAmount::default(),
        quality_adj_power_smoothed: FilterEstimate::new(rwd, TokenAmount::default()),
    };
    let current_reward = ThisEpochRewardReturn {
        this_epoch_baseline_power: power,
        this_epoch_reward_smoothed: epoch_reward_smooth,
    };
    rt.expect_send(
        *REWARD_ACTOR_ADDR,
        RewardMethod::ThisEpochReward as u64,
        RawBytes::default(),
        TokenAmount::from(0u8),
        RawBytes::serialize(current_reward).unwrap(),
        ExitCode::OK,
    );
    rt.expect_send(
        *STORAGE_POWER_ACTOR_ADDR,
        PowerMethod::CurrentTotalPower as u64,
        RawBytes::default(),
        TokenAmount::from(0u8),
        RawBytes::serialize(current_power).unwrap(),
        ExitCode::OK,
    );
}

fn assert_n_good_deals<BS>(dobe: &SetMultimap<BS>, epoch: ChainEpoch, n: isize)
where
    BS: fvm_ipld_blockstore::Blockstore,
{
    let deal_updates_interval = Policy::default().deal_updates_interval;
    let mut count = 0;
    dobe.for_each(epoch, |id| {
        assert_eq!(epoch % deal_updates_interval, (id as i64) % deal_updates_interval);
        count += 1;
        Ok(())
    })
    .unwrap();
    assert_eq!(n, count, "unexpected deal count at epoch {}", epoch);
}

fn assert_deals_terminated(rt: &mut MockRuntime, epoch: ChainEpoch, deal_ids: &[DealID]) {
    for &deal_id in deal_ids {
        let s = get_deal_state(rt, deal_id);
        assert_eq!(s.slash_epoch, epoch);
    }
}

fn assert_deals_not_terminated(rt: &mut MockRuntime, deal_ids: &[DealID]) {
    for &deal_id in deal_ids {
        let s = get_deal_state(rt, deal_id);
        assert_eq!(s.slash_epoch, EPOCH_UNDEFINED);
    }
}

fn assert_deal_deleted(rt: &mut MockRuntime, deal_id: DealID, p: DealProposal) {
    use cid::multihash::Code;
    use cid::multihash::MultihashDigest;
    use fvm_ipld_hamt::{BytesKey, Hamt};

    let st: State = rt.get_state();

    // Check that the deal_id is not in st.proposals.
    let deals = DealArray::load(&st.proposals, &rt.store).unwrap();
    let d = deals.get(deal_id).unwrap();
    assert!(d.is_none());

    // Check that the deal_id is not in st.states
    let states = DealMetaArray::load(&st.states, &rt.store).unwrap();
    let s = states.get(deal_id).unwrap();
    assert!(s.is_none());

    let mh_code = Code::Blake2b256;
    let p_cid = Cid::new_v1(fvm_ipld_encoding::DAG_CBOR, mh_code.digest(&to_vec(&p).unwrap()));
    // Check that the deal_id is not in st.pending_proposals.
    let pending_deals: Hamt<&fvm_ipld_blockstore::MemoryBlockstore, DealProposal> =
        fil_actors_runtime::make_map_with_root_and_bitwidth(
            &st.pending_proposals,
            &rt.store,
            PROPOSALS_AMT_BITWIDTH,
        )
        .unwrap();
    assert!(!pending_deals.contains_key(&BytesKey(p_cid.to_bytes())).unwrap());
}

fn process_epoch(start_epoch: ChainEpoch, deal_id: DealID) -> ChainEpoch {
    let policy = Policy::default();
    gen_rand_next_epoch(&policy, start_epoch, deal_id)
}
