// Copyright 2019-2022 ChainSafe Systems
// SPDX-License-Identifier: Apache-2.0, MIT

use std::cell::RefCell;

use fil_actor_cron::testing::check_state_invariants;
use fil_actor_cron::{Actor as CronActor, ConstructorParams, Entry, State};
use fil_actors_runtime::test_utils::*;
use fil_actors_runtime::SYSTEM_ACTOR_ADDR;
use fvm_ipld_encoding::ipld_block::IpldBlock;
use fvm_shared::address::Address;
use fvm_shared::econ::TokenAmount;
use fvm_shared::error::ExitCode;
use num_traits::Zero;

fn check_state(rt: &MockRuntime) {
    let (_, acc) = check_state_invariants(&rt.get_state());
    acc.assert_empty();
}

fn construct_runtime() -> MockRuntime {
    MockRuntime {
        receiver: Address::new_id(100),
        caller: RefCell::new(SYSTEM_ACTOR_ADDR),
        caller_type: RefCell::new(*SYSTEM_ACTOR_CODE_ID),
        ..Default::default()
    }
}

#[test]
fn construct_with_empty_entries() {
    let rt = construct_runtime();

    construct_and_verify(&rt, &ConstructorParams { entries: vec![] });
    let state: State = rt.get_state();

    assert_eq!(state.entries, vec![]);
    check_state(&rt);
}

#[test]
fn construct_with_entries() {
    let rt = construct_runtime();

    let entry1 = Entry { receiver: Address::new_id(1001), method_num: 1001 };
    let entry2 = Entry { receiver: Address::new_id(1002), method_num: 1002 };
    let entry3 = Entry { receiver: Address::new_id(1003), method_num: 1003 };
    let entry4 = Entry { receiver: Address::new_id(1004), method_num: 1004 };

    let params = ConstructorParams { entries: vec![entry1, entry2, entry3, entry4] };

    construct_and_verify(&rt, &params);

    let state: State = rt.get_state();

    assert_eq!(state.entries, params.entries);
    check_state(&rt);
}

#[test]
fn epoch_tick_with_empty_entries() {
    let rt = construct_runtime();

    construct_and_verify(&rt, &ConstructorParams { entries: vec![] });
    epoch_tick_and_verify(&rt);
}

#[test]
fn epoch_tick_with_entries() {
    let rt = construct_runtime();

    let entry1 = Entry { receiver: Address::new_id(1001), method_num: 1001 };
    let entry2 = Entry { receiver: Address::new_id(1002), method_num: 1002 };
    let entry3 = Entry { receiver: Address::new_id(1003), method_num: 1003 };
    let entry4 = Entry { receiver: Address::new_id(1004), method_num: 1004 };

    let params = ConstructorParams {
        entries: vec![entry1.clone(), entry2.clone(), entry3.clone(), entry4.clone()],
    };

    construct_and_verify(&rt, &params);

    // ExitCodes dont matter here
    rt.expect_send_simple(
        entry1.receiver,
        entry1.method_num,
        None,
        TokenAmount::zero(),
        None,
        ExitCode::OK,
    );
    rt.expect_send_simple(
        entry2.receiver,
        entry2.method_num,
        None,
        TokenAmount::zero(),
        None,
        ExitCode::USR_ILLEGAL_ARGUMENT,
    );
    rt.expect_send_simple(
        entry3.receiver,
        entry3.method_num,
        None,
        TokenAmount::zero(),
        None,
        ExitCode::OK,
    );
    rt.expect_send_simple(
        entry4.receiver,
        entry4.method_num,
        None,
        TokenAmount::zero(),
        None,
        ExitCode::OK,
    );

    epoch_tick_and_verify(&rt);
}

fn construct_and_verify(rt: &MockRuntime, params: &ConstructorParams) {
    rt.set_caller(*SYSTEM_ACTOR_CODE_ID, SYSTEM_ACTOR_ADDR);
    rt.expect_validate_caller_addr(vec![SYSTEM_ACTOR_ADDR]);
    let ret = rt.call::<CronActor>(1, IpldBlock::serialize_cbor(&params).unwrap()).unwrap();
    assert!(ret.is_none());
    rt.verify();
}

fn epoch_tick_and_verify(rt: &MockRuntime) {
    rt.expect_validate_caller_addr(vec![SYSTEM_ACTOR_ADDR]);
    let ret = rt.call::<CronActor>(2, None).unwrap();
    assert!(ret.is_none());
    rt.verify();
    check_state(rt);
}
