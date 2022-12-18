// Copyright 2019-2022 ChainSafe Systems
// SPDX-License-Identifier: Apache-2.0, MIT

use cid::Cid;
use fil_actor_init::testing::check_state_invariants;
use fil_actor_init::{
    Actor as InitActor, ConstructorParams, Exec4Params, Exec4Return, ExecParams, ExecReturn,
    Method, State,
};
use fil_actors_runtime::runtime::Runtime;
use fil_actors_runtime::{test_utils::*, EAM_ACTOR_ADDR, EAM_ACTOR_ID};
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
        rt.expect_create_actor(*MULTISIG_ACTOR_CODE_ID, expected_id, None);

        // Expect a send to the multisig actor constructor
        rt.expect_send(
            expected_id_addr,
            METHOD_CONSTRUCTOR,
            IpldBlock::serialize_cbor(&fake_params).unwrap(),
            TokenAmount::zero(),
            RawBytes::default(),
            ExitCode::OK,
        );

        // Return should have been successful. Check the returned addresses
        let exec_ret = exec_and_verify(&mut rt, *MULTISIG_ACTOR_CODE_ID, &fake_params).unwrap();
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
        rt.expect_create_actor(*PAYCH_ACTOR_CODE_ID, expected_id, None);

        let fake_params = ConstructorParams { network_name: String::from("fake_param") };

        // expect anne creating a payment channel to trigger a send to the payment channels constructor
        let balance = TokenAmount::from_atto(100);

        rt.expect_send(
            expected_id_addr,
            METHOD_CONSTRUCTOR,
            IpldBlock::serialize_cbor(&fake_params).unwrap(),
            balance,
            RawBytes::default(),
            ExitCode::OK,
        );

        let exec_ret = exec_and_verify(&mut rt, *PAYCH_ACTOR_CODE_ID, &fake_params).unwrap();
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
    rt.expect_create_actor(*MINER_ACTOR_CODE_ID, expected_id, None);

    let fake_params = ConstructorParams { network_name: String::from("fake_param") };

    rt.expect_send(
        expected_id_addr,
        METHOD_CONSTRUCTOR,
        IpldBlock::serialize_cbor(&fake_params).unwrap(),
        TokenAmount::zero(),
        RawBytes::default(),
        ExitCode::OK,
    );

    let exec_ret = exec_and_verify(&mut rt, *MINER_ACTOR_CODE_ID, &fake_params).unwrap();
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
    rt.expect_create_actor(*MULTISIG_ACTOR_CODE_ID, expected_id, None);

    let fake_params = ConstructorParams { network_name: String::from("fake_param") };
    // Expect a send to the multisig actor constructor
    rt.expect_send(
        expected_id_addr,
        METHOD_CONSTRUCTOR,
        IpldBlock::serialize_cbor(&fake_params).unwrap(),
        TokenAmount::zero(),
        RawBytes::default(),
        ExitCode::OK,
    );

    // Return should have been successful. Check the returned addresses
    let exec_ret = exec_and_verify(&mut rt, *MULTISIG_ACTOR_CODE_ID, &fake_params).unwrap();
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
    rt.expect_create_actor(*MINER_ACTOR_CODE_ID, expected_id, None);

    let fake_params = ConstructorParams { network_name: String::from("fake_param") };
    rt.expect_send(
        expected_id_addr,
        METHOD_CONSTRUCTOR,
        IpldBlock::serialize_cbor(&fake_params).unwrap(),
        TokenAmount::zero(),
        RawBytes::default(),
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
    rt.set_caller(*EVM_ACTOR_CODE_ID, Address::new_id(1000));

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
            IpldBlock::serialize_cbor(&exec_params).unwrap(),
        ),
    );

    // can call the exported method num

    // Assign addresses
    let unique_address = Address::new_actor(b"multisig");
    rt.new_actor_addr = Some(unique_address);

    // Next id
    let expected_id = 100;
    let expected_id_addr = Address::new_id(expected_id);
    rt.expect_create_actor(*MULTISIG_ACTOR_CODE_ID, expected_id, None);

    // Expect a send to the multisig actor constructor
    rt.expect_send(
        expected_id_addr,
        METHOD_CONSTRUCTOR,
        IpldBlock::serialize_cbor(&fake_constructor_params).unwrap(),
        TokenAmount::zero(),
        RawBytes::default(),
        ExitCode::OK,
    );

    rt.expect_validate_caller_any();

    let ret = rt
        .call::<InitActor>(
            Method::ExecExported as u64,
            IpldBlock::serialize_cbor(&exec_params).unwrap(),
        )
        .unwrap();
    let exec_ret: ExecReturn = RawBytes::deserialize(&ret).unwrap();
    assert_eq!(unique_address, exec_ret.robust_address, "Robust address does not macth");
    assert_eq!(expected_id_addr, exec_ret.id_address, "Id address does not match");
    check_state(&rt);
    rt.verify();
}

#[test]
fn call_exec4() {
    let mut rt = construct_runtime();
    construct_and_verify(&mut rt);

    // Assign addresses
    let unique_address = Address::new_actor(b"test");
    rt.new_actor_addr = Some(unique_address);

    // Make the f4 addr
    let subaddr = b"foobar";
    let f4_addr = Address::new_delegated(EAM_ACTOR_ID, subaddr).unwrap();

    // Next id
    let expected_id = 100;
    let expected_id_addr = Address::new_id(expected_id);
    rt.expect_create_actor(*MULTISIG_ACTOR_CODE_ID, expected_id, Some(f4_addr));

    let fake_params = ConstructorParams { network_name: String::from("fake_param") };
    // Expect a send to the multisig actor constructor
    rt.expect_send(
        expected_id_addr,
        METHOD_CONSTRUCTOR,
        IpldBlock::serialize_cbor(&fake_params).unwrap(),
        TokenAmount::zero(),
        RawBytes::default(),
        ExitCode::OK,
    );

    // Return should have been successful. Check the returned addresses
    let exec_ret =
        exec4_and_verify(&mut rt, subaddr, *MULTISIG_ACTOR_CODE_ID, &fake_params).unwrap();

    assert_eq!(unique_address, exec_ret.robust_address, "Robust address does not macth");
    assert_eq!(expected_id_addr, exec_ret.id_address, "Id address does not match");

    // Check that we assigned the right f4 address.
    let init_state: State = rt.get_state();
    let resolved_id = init_state
        .resolve_address(rt.store(), &f4_addr)
        .ok()
        .flatten()
        .expect("failed to lookup f4 address");
    assert_eq!(expected_id_addr, resolved_id, "f4 address not assigned to the right actor");

    // Try again and expect it to fail with "forbidden".
    let unique_address = Address::new_actor(b"test2");
    rt.new_actor_addr = Some(unique_address);
    let exec_err =
        exec4_and_verify(&mut rt, subaddr, *MULTISIG_ACTOR_CODE_ID, &fake_params).unwrap_err();

    assert_eq!(exec_err.exit_code(), ExitCode::USR_FORBIDDEN);

    // Delete and try again, it should still fail.
    rt.actor_code_cids.remove(&resolved_id);
    let unique_address = Address::new_actor(b"test2");
    rt.new_actor_addr = Some(unique_address);
    let exec_err =
        exec4_and_verify(&mut rt, subaddr, *MULTISIG_ACTOR_CODE_ID, &fake_params).unwrap_err();

    assert_eq!(exec_err.exit_code(), ExitCode::USR_FORBIDDEN);
}

// Try turning an embryo into an f4 actor.
#[test]
fn call_exec4_embryo() {
    let mut rt = construct_runtime();
    construct_and_verify(&mut rt);

    // Assign addresses
    let unique_address = Address::new_actor(b"test");
    rt.new_actor_addr = Some(unique_address);

    // Make the f4 addr
    let subaddr = b"foobar";
    let f4_addr = Address::new_delegated(EAM_ACTOR_ID, subaddr).unwrap();

    // Register an embryo with the init actor.
    let expected_id = {
        let mut state: State = rt.get_state();
        let (id, existing) = state.map_addresses_to_id(rt.store(), &f4_addr, None).unwrap();
        assert!(!existing);
        rt.replace_state(&state);
        id
    };

    // Register it in the state-tree.
    let expected_id_addr = Address::new_id(expected_id);
    rt.set_address_actor_type(expected_id_addr, *EMBRYO_ACTOR_CODE_ID);
    rt.add_delegated_address(expected_id_addr, f4_addr);

    // Now try to create it.
    rt.expect_create_actor(*MULTISIG_ACTOR_CODE_ID, expected_id, Some(f4_addr));

    let fake_params = ConstructorParams { network_name: String::from("fake_param") };
    // Expect a send to the multisig actor constructor
    rt.expect_send(
        expected_id_addr,
        METHOD_CONSTRUCTOR,
        IpldBlock::serialize_cbor(&fake_params).unwrap(),
        TokenAmount::zero(),
        RawBytes::default(),
        ExitCode::OK,
    );

    // Return should have been successful. Check the returned addresses
    let exec_ret =
        exec4_and_verify(&mut rt, subaddr, *MULTISIG_ACTOR_CODE_ID, &fake_params).unwrap();

    assert_eq!(unique_address, exec_ret.robust_address, "Robust address does not macth");
    assert_eq!(expected_id_addr, exec_ret.id_address, "Id address does not match");

    // Check that we assigned the right f4 address.
    let init_state: State = rt.get_state();
    let resolved_id = init_state
        .resolve_address(rt.store(), &f4_addr)
        .ok()
        .flatten()
        .expect("failed to lookup f4 address");
    assert_eq!(expected_id_addr, resolved_id, "f4 address not assigned to the right actor");
}

fn construct_and_verify(rt: &mut MockRuntime) {
    rt.set_caller(*SYSTEM_ACTOR_CODE_ID, SYSTEM_ACTOR_ADDR);
    rt.expect_validate_caller_addr(vec![SYSTEM_ACTOR_ADDR]);
    let params = ConstructorParams { network_name: "mock".to_string() };
    let ret = rt
        .call::<InitActor>(METHOD_CONSTRUCTOR, IpldBlock::serialize_cbor(&params).unwrap())
        .unwrap();

    assert_eq!(RawBytes::default(), ret);
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
) -> Result<ExecReturn, ActorError>
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
    ret.and_then(|v| v.deserialize().map_err(|e| e.into()))
}

fn exec4_and_verify<S: Serialize>(
    rt: &mut MockRuntime,
    subaddr: &[u8],
    code_id: Cid,
    params: &S,
) -> Result<Exec4Return, ActorError>
where
    S: Serialize,
{
    rt.set_caller(*ACCOUNT_ACTOR_CODE_ID, EAM_ACTOR_ADDR);
    if cfg!(feature = "m2-native") {
        rt.expect_validate_caller_any();
    } else {
        rt.expect_validate_caller_addr(vec![EAM_ACTOR_ADDR]);
    }
    let exec_params = Exec4Params {
        code_cid: code_id,
        constructor_params: RawBytes::serialize(params).unwrap(),
        subaddress: subaddr.to_owned().into(),
    };

    let ret = rt
        .call::<InitActor>(Method::Exec4 as u64, IpldBlock::serialize_cbor(&exec_params).unwrap());

    rt.verify();
    check_state(rt);
    ret.and_then(|v| v.deserialize().map_err(|e| e.into()))
}
