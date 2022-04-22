// Copyright 2019-2022 ChainSafe Systems
// SPDX-License-Identifier: Apache-2.0, MIT

use std::collections::HashMap;

use fil_actor_market::balance_table::{BalanceTable, BALANCE_TABLE_BITWIDTH};
use fil_actor_market::{
    ext, ActivateDealsParams, Actor as MarketActor, ClientDealProposal, DealArray, DealMetaArray,
    DealProposal, DealState, Label, Method, OnMinerSectorsTerminateParams,
    PublishStorageDealsParams, PublishStorageDealsReturn, State, WithdrawBalanceParams,
    WithdrawBalanceReturn, PROPOSALS_AMT_BITWIDTH, STATES_AMT_BITWIDTH,
};
use fil_actor_power::{CurrentTotalPowerReturn, Method as PowerMethod};
use fil_actor_reward::Method as RewardMethod;
use fil_actor_verifreg::UseBytesParams;
use fil_actors_runtime::cbor::deserialize;
use fil_actors_runtime::network::EPOCHS_IN_DAY;
use fil_actors_runtime::runtime::{Policy, Runtime};
use fil_actors_runtime::test_utils::*;
use fil_actors_runtime::{
    make_empty_map, ActorError, SetMultimap, REWARD_ACTOR_ADDR, STORAGE_MARKET_ACTOR_ADDR,
    STORAGE_POWER_ACTOR_ADDR, SYSTEM_ACTOR_ADDR, VERIFIED_REGISTRY_ACTOR_ADDR,
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
    use std::convert::TryInto;
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

fn get_deal_state(rt: &mut MockRuntime, deal_id: DealID) -> DealState {
    let st: State = rt.get_state();

    let states = DealMetaArray::load(&st.states, &rt.store).unwrap();

    let s = states.get(deal_id).unwrap();
    *s.unwrap()
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
    rt.set_caller(*MINER_ACTOR_CODE_ID, miner_addr);
    rt.expect_validate_caller_type(vec![*MINER_ACTOR_CODE_ID]);

    let params = OnMinerSectorsTerminateParams { epoch: rt.epoch, deal_ids: deal_ids.to_vec() };

    let ret = rt
        .call::<MarketActor>(
            Method::OnMinerSectorsTerminate as u64,
            &RawBytes::serialize(params).unwrap(),
        )
        .unwrap();
    assert_eq!(ret, RawBytes::default());
    rt.verify();
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
