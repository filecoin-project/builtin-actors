// Copyright 2019-2022 ChainSafe Systems
// SPDX-License-Identifier: Apache-2.0, MIT

use cid::Cid;
use fil_actor_init::testing::check_state_invariants;
use fil_actor_init::{
    Actor as InitActor, ConstructorParams, ExecParams, ExecReturn, Method, State,
};
use fil_actors_runtime::cbor::serialize;
use fil_actors_runtime::runtime::Runtime;
use fil_actors_runtime::test_utils::*;
use fil_actors_runtime::{
    ActorError, Multimap, FIRST_NON_SINGLETON_ADDR, STORAGE_POWER_ACTOR_ADDR, SYSTEM_ACTOR_ADDR,
};
use fvm_ipld_encoding::ipld_block::IpldBlock;
use fvm_ipld_encoding::RawBytes;
use fvm_shared::address::Address;
use fvm_shared::econ::TokenAmount;
use fvm_shared::error::ExitCode;
use fvm_shared::{MethodNum, HAMT_BIT_WIDTH, METHOD_CONSTRUCTOR};
use num_traits::Zero;
use serde::Serialize;

fn check_state(rt: &MockRuntime) {
    let (_, acc) = check_state_invariants(&rt.get_state(), rt.store());
    acc.assert_empty();
}

fn construct_runtime() -> MockRuntime {
    MockRuntime {
        receiver: Address::new_id(1000),
        caller: SYSTEM_ACTOR_ADDR,
        caller_type: *SYSTEM_ACTOR_CODE_ID,
        ..Default::default()
    }
}

// Test to make sure we abort actors that can not call the exec function
#[test]
fn abort_cant_call_exec() {
    let mut rt = construct_runtime();
    construct_and_verify(&mut rt);
    let anne = Address::new_id(1001);

    rt.set_caller(*ACCOUNT_ACTOR_CODE_ID, anne);

    let err =
        exec_and_verify(&mut rt, *POWER_ACTOR_CODE_ID, &"").expect_err("Exec should have failed");
    assert_eq!(err.exit_code(), ExitCode::USR_FORBIDDEN);
    check_state(&rt);
}

#[test]
fn repeated_robust_address() {
    let mut rt = construct_runtime();
    construct_and_verify(&mut rt);

    // setup one msig actor
    let unique_address = Address::new_actor(b"multisig");
    let fake_params = ConstructorParams { network_name: String::from("fake_param") };
    {
        // Actor creating multisig actor
        let some_acc_actor = Address::new_id(1234);
        rt.set_caller(*ACCOUNT_ACTOR_CODE_ID, some_acc_actor);

        rt.new_actor_addr = Some(unique_address);

        // Next id
        let expected_id = 100;
        let expected_id_addr = Address::new_id(expected_id);
        rt.expect_create_actor(*MULTISIG_ACTOR_CODE_ID, expected_id);

        // Expect a send to the multisig actor constructor
        rt.expect_send(
            expected_id_addr,
            METHOD_CONSTRUCTOR,
            IpldBlock::serialize_cbor(&fake_params).unwrap(),
            TokenAmount::zero(),
            None,
            ExitCode::OK,
        );

        // Return should have been successful. Check the returned addresses
        let exec_ret =
            exec_and_verify(&mut rt, *MULTISIG_ACTOR_CODE_ID, &fake_params).unwrap().unwrap();
        let exec_ret: ExecReturn = exec_ret.deserialize().unwrap();
        assert_eq!(unique_address, exec_ret.robust_address, "Robust address does not macth");
        assert_eq!(expected_id_addr, exec_ret.id_address, "Id address does not match");
        check_state(&rt);
    }

    // Simulate repeated robust address, as it could be a case with predictable address generation
    {
        rt.new_actor_addr = Some(unique_address);

        rt.expect_validate_caller_any();
        let exec_params = ExecParams {
            code_cid: *MULTISIG_ACTOR_CODE_ID,
            constructor_params: RawBytes::serialize(&fake_params).unwrap(),
        };

        let ret = rt.call::<InitActor>(
            Method::Exec as u64,
            IpldBlock::serialize_cbor(&exec_params).unwrap(),
        );

        rt.verify();
        assert!(ret.is_err());
        assert_eq!(ret.unwrap_err().exit_code(), ExitCode::USR_FORBIDDEN)
    }
}

#[test]
fn create_2_payment_channels() {
    let mut rt = construct_runtime();
    construct_and_verify(&mut rt);
    let anne = Address::new_id(1001);

    rt.set_caller(*ACCOUNT_ACTOR_CODE_ID, anne);

    for n in 0..2 {
        let pay_channel_string = format!("paych_{}", n);
        let paych = pay_channel_string.as_bytes();

        rt.set_balance(TokenAmount::from_atto(100));
        rt.value_received = TokenAmount::from_atto(100);

        let unique_address = Address::new_actor(paych);
        rt.new_actor_addr = Some(Address::new_actor(paych));

        let expected_id = 100 + n;
        let expected_id_addr = Address::new_id(expected_id);
        rt.expect_create_actor(*PAYCH_ACTOR_CODE_ID, expected_id);

        let fake_params = ConstructorParams { network_name: String::from("fake_param") };

        // expect anne creating a payment channel to trigger a send to the payment channels constructor
        let balance = TokenAmount::from_atto(100u8);

        rt.expect_send(
            expected_id_addr,
            METHOD_CONSTRUCTOR,
            IpldBlock::serialize_cbor(&fake_params).unwrap(),
            balance,
            None,
            ExitCode::OK,
        );

        let exec_ret =
            exec_and_verify(&mut rt, *PAYCH_ACTOR_CODE_ID, &fake_params).unwrap().unwrap();
        let exec_ret: ExecReturn = exec_ret.deserialize().unwrap();
        assert_eq!(unique_address, exec_ret.robust_address, "Robust Address does not match");
        assert_eq!(expected_id_addr, exec_ret.id_address, "Id address does not match");

        let state: State = rt.get_state();
        let returned_address = state
            .resolve_address(&rt.store, &unique_address)
            .expect("Resolve should not error")
            .expect("Address should be able to be resolved");

        assert_eq!(returned_address, expected_id_addr, "Wrong Address returned");
        check_state(&rt);
    }
}

#[test]
fn create_storage_miner() {
    let mut rt = construct_runtime();
    construct_and_verify(&mut rt);

    // only the storage power actor can create a miner
    rt.set_caller(*POWER_ACTOR_CODE_ID, STORAGE_POWER_ACTOR_ADDR);

    let unique_address = Address::new_actor(b"miner");
    rt.new_actor_addr = Some(unique_address);

    let expected_id = 100;
    let expected_id_addr = Address::new_id(expected_id);
    rt.expect_create_actor(*MINER_ACTOR_CODE_ID, expected_id);

    let fake_params = ConstructorParams { network_name: String::from("fake_param") };

    rt.expect_send(
        expected_id_addr,
        METHOD_CONSTRUCTOR,
        IpldBlock::serialize_cbor(&fake_params).unwrap(),
        TokenAmount::zero(),
        None,
        ExitCode::OK,
    );

    let exec_ret = exec_and_verify(&mut rt, *MINER_ACTOR_CODE_ID, &fake_params).unwrap().unwrap();

    let exec_ret: ExecReturn = exec_ret.deserialize().unwrap();
    assert_eq!(unique_address, exec_ret.robust_address);
    assert_eq!(expected_id_addr, exec_ret.id_address);

    // Address should be resolved
    let state: State = rt.get_state();
    let returned_address = state
        .resolve_address(&rt.store, &unique_address)
        .expect("Resolve should not error")
        .expect("Address should be able to be resolved");
    assert_eq!(expected_id_addr, returned_address);

    // Should return error since the address of flurbo is unknown
    let unknown_addr = Address::new_actor(b"flurbo");

    let returned_address = state.resolve_address(&rt.store, &unknown_addr).unwrap();
    assert_eq!(returned_address, None, "Addresses should have not been found");
    check_state(&rt);
}

#[test]
fn create_multisig_actor() {
    let mut rt = construct_runtime();
    construct_and_verify(&mut rt);

    // Actor creating multisig actor
    let some_acc_actor = Address::new_id(1234);
    rt.set_caller(*ACCOUNT_ACTOR_CODE_ID, some_acc_actor);

    // Assign addresses
    let unique_address = Address::new_actor(b"multisig");
    rt.new_actor_addr = Some(unique_address);

    // Next id
    let expected_id = 100;
    let expected_id_addr = Address::new_id(expected_id);
    rt.expect_create_actor(*MULTISIG_ACTOR_CODE_ID, expected_id);

    let fake_params = ConstructorParams { network_name: String::from("fake_param") };
    // Expect a send to the multisig actor constructor
    rt.expect_send(
        expected_id_addr,
        METHOD_CONSTRUCTOR,
        IpldBlock::serialize_cbor(&fake_params).unwrap(),
        TokenAmount::zero(),
        None,
        ExitCode::OK,
    );

    // Return should have been successful. Check the returned addresses
    let exec_ret =
        exec_and_verify(&mut rt, *MULTISIG_ACTOR_CODE_ID, &fake_params).unwrap().unwrap();
    let exec_ret: ExecReturn = exec_ret.deserialize().unwrap();
    assert_eq!(unique_address, exec_ret.robust_address, "Robust address does not macth");
    assert_eq!(expected_id_addr, exec_ret.id_address, "Id address does not match");
    check_state(&rt);
}

#[test]
fn sending_constructor_failure() {
    let mut rt = construct_runtime();
    construct_and_verify(&mut rt);

    // Only the storage power actor can create a miner
    rt.set_caller(*POWER_ACTOR_CODE_ID, STORAGE_POWER_ACTOR_ADDR);

    // Assign new address for the storage actor miner
    let unique_address = Address::new_actor(b"miner");
    rt.new_actor_addr = Some(unique_address);

    // Create the next id address
    let expected_id = 100;
    let expected_id_addr = Address::new_id(expected_id);
    rt.expect_create_actor(*MINER_ACTOR_CODE_ID, expected_id);

    let fake_params = ConstructorParams { network_name: String::from("fake_param") };
    rt.expect_send(
        expected_id_addr,
        METHOD_CONSTRUCTOR,
        IpldBlock::serialize_cbor(&fake_params).unwrap(),
        TokenAmount::zero(),
        None,
        ExitCode::USR_ILLEGAL_STATE,
    );

    let error = exec_and_verify(&mut rt, *MINER_ACTOR_CODE_ID, &fake_params)
        .expect_err("sending constructor should have failed");

    let error_exit_code = error.exit_code();

    assert_eq!(
        error_exit_code,
        ExitCode::USR_ILLEGAL_STATE,
        "Exit Code that is returned is not ErrIllegalState"
    );

    let state: State = rt.get_state();

    let returned_address = state.resolve_address(&rt.store, &unique_address).unwrap();
    assert_eq!(returned_address, None, "Addresses should have not been found");
    check_state(&rt);
}

#[test]
fn exec_restricted_correctly() {
    let mut rt = construct_runtime();
    construct_and_verify(&mut rt);

    // set caller to not-builtin
    rt.set_caller(make_identity_cid(b"1234"), Address::new_id(1000));

    // cannot call the unexported method num
    let fake_constructor_params =
        RawBytes::serialize(ConstructorParams { network_name: String::from("fake_param") })
            .unwrap();
    let exec_params = ExecParams {
        code_cid: *MULTISIG_ACTOR_CODE_ID,
        constructor_params: RawBytes::serialize(fake_constructor_params.clone()).unwrap(),
    };

    expect_abort_contains_message(
        ExitCode::USR_FORBIDDEN,
        "must be built-in",
        rt.call::<InitActor>(
            Method::Exec as MethodNum,
            &serialize(&exec_params, "params").unwrap(),
        ),
    );

    // can call the exported method num

    // Assign addresses
    let unique_address = Address::new_actor(b"multisig");
    rt.new_actor_addr = Some(unique_address);

    // Next id
    let expected_id = 100;
    let expected_id_addr = Address::new_id(expected_id);
    rt.expect_create_actor(*MULTISIG_ACTOR_CODE_ID, expected_id);

    // Expect a send to the multisig actor constructor
    rt.expect_send(
        expected_id_addr,
        METHOD_CONSTRUCTOR,
        RawBytes::serialize(&fake_constructor_params).unwrap(),
        TokenAmount::zero(),
        RawBytes::default(),
        ExitCode::OK,
    );

    rt.expect_validate_caller_any();

    let ret = rt
        .call::<InitActor>(Method::ExecExported as u64, &RawBytes::serialize(&exec_params).unwrap())
        .unwrap();
    let exec_ret: ExecReturn = RawBytes::deserialize(&ret).unwrap();
    assert_eq!(unique_address, exec_ret.robust_address, "Robust address does not macth");
    assert_eq!(expected_id_addr, exec_ret.id_address, "Id address does not match");
    check_state(&rt);
    rt.verify();
}

fn construct_and_verify(rt: &mut MockRuntime) {
    rt.set_caller(*SYSTEM_ACTOR_CODE_ID, SYSTEM_ACTOR_ADDR);
    rt.expect_validate_caller_addr(vec![SYSTEM_ACTOR_ADDR]);
    let params = ConstructorParams { network_name: "mock".to_string() };
    let ret = rt
        .call::<InitActor>(METHOD_CONSTRUCTOR, IpldBlock::serialize_cbor(&params).unwrap())
        .unwrap();

    assert!(ret.is_none());

    rt.verify();

    let state_data: State = rt.get_state();

    // Gets the Result(CID)
    let empty_map =
        Multimap::from_root(&rt.store, &state_data.address_map, HAMT_BIT_WIDTH, 3).unwrap().root();

    assert_eq!(empty_map.unwrap(), state_data.address_map);
    assert_eq!(FIRST_NON_SINGLETON_ADDR, state_data.next_id);
    assert_eq!("mock".to_string(), state_data.network_name);
    check_state(rt);
}

fn exec_and_verify<S: Serialize>(
    rt: &mut MockRuntime,
    code_id: Cid,
    params: &S,
) -> Result<Option<IpldBlock>, ActorError>
where
    S: Serialize,
{
    rt.expect_validate_caller_any();
    let exec_params =
        ExecParams { code_cid: code_id, constructor_params: RawBytes::serialize(params).unwrap() };

    let ret =
        rt.call::<InitActor>(Method::Exec as u64, IpldBlock::serialize_cbor(&exec_params).unwrap());

    rt.verify();
    check_state(rt);
    ret
}
