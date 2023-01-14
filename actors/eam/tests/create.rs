use eam::ext::evm::RESURRECT_METHOD;
use eam::ext::init::{Exec4Params, Exec4Return, EXEC4_METHOD};
use eam::{
    compute_address_create, Create2Params, CreateParams, EthAddress, EvmConstructorParams, Return,
};
use fil_actor_eam as eam;
use fil_actor_eam::CreateExternalParams;
use fil_actors_runtime::runtime::builtins::Type;
use fil_actors_runtime::runtime::Primitives;
use fil_actors_runtime::test_utils::{
    expect_empty, MockRuntime, ETHACCOUNT_ACTOR_CODE_ID, EVM_ACTOR_CODE_ID,
    PLACEHOLDER_ACTOR_CODE_ID, SYSTEM_ACTOR_CODE_ID,
};
use fil_actors_runtime::{INIT_ACTOR_ADDR, SYSTEM_ACTOR_ADDR};
use fvm_ipld_encoding::ipld_block::IpldBlock;
use fvm_ipld_encoding::RawBytes;
use fvm_shared::address::Address;
use fvm_shared::econ::TokenAmount;
use fvm_shared::error::ExitCode;

#[test]
fn call_create_new() {
    let mut rt = construct_and_verify();

    let id_addr = Address::new_id(110);
    let eth_addr = eam::EthAddress(hex_literal::hex!("CAFEB0BA00000000000000000000000000000000"));
    let f4_eth_addr = Address::new_delegated(10, &eth_addr.0).unwrap();
    rt.add_delegated_address(id_addr, f4_eth_addr);

    rt.set_caller(*EVM_ACTOR_CODE_ID, id_addr);

    rt.expect_validate_caller_type(vec![Type::EVM]);

    let initcode = vec![0xff];

    let create_params = CreateParams { initcode: initcode.clone(), nonce: 0 };

    let evm_params = EvmConstructorParams { creator: eth_addr, initcode: initcode.into() };

    let new_eth_addr = compute_address_create(&rt, &eth_addr, 0);
    let params = Exec4Params {
        code_cid: *EVM_ACTOR_CODE_ID,
        constructor_params: RawBytes::serialize(evm_params).unwrap(),
        subaddress: new_eth_addr.0[..].to_owned().into(),
    };

    let send_return = IpldBlock::serialize_cbor(&Exec4Return {
        id_address: Address::new_id(111),
        robust_address: Address::new_id(0), // not a robust address but im hacking here and nobody checks
    })
    .unwrap();

    rt.expect_send(
        INIT_ACTOR_ADDR,
        EXEC4_METHOD,
        IpldBlock::serialize_cbor(&params).unwrap(),
        TokenAmount::from_atto(0),
        send_return,
        ExitCode::OK,
    );

    let result = rt
        .call::<eam::EamActor>(
            eam::Method::Create as u64,
            IpldBlock::serialize_cbor(&create_params).unwrap(),
        )
        .unwrap()
        .unwrap()
        .deserialize::<Return>()
        .unwrap();

    let expected_return = Return {
        actor_id: 111,
        robust_address: Some(Address::new_id(0)),
        eth_address: new_eth_addr,
    };

    assert_eq!(result, expected_return);
    rt.verify();
}

#[test]
fn call_create_external_over_placeholder() {
    let mut rt = construct_and_verify();

    let caller_id_addr = Address::new_id(110);
    let caller_eth_addr =
        eam::EthAddress(hex_literal::hex!("CAFEB0BA00000000000000000000000000000000"));
    let caller_f4_eth_addr = Address::new_delegated(10, &caller_eth_addr.0).unwrap();
    rt.add_delegated_address(caller_id_addr, caller_f4_eth_addr);

    // Accounts and EthAccounts are the valid caller types for CreateExternal
    rt.set_caller(*ETHACCOUNT_ACTOR_CODE_ID, caller_id_addr);
    rt.set_origin(caller_id_addr);

    let target_id_addr = Address::new_id(111);
    let target_eth_addr = compute_address_create(&rt, &caller_eth_addr, 0);
    let target_f4_eth_addr = Address::new_delegated(10, &target_eth_addr.0).unwrap();
    rt.add_delegated_address(target_id_addr, target_f4_eth_addr);
    rt.set_address_actor_type(target_id_addr, *PLACEHOLDER_ACTOR_CODE_ID);

    let initcode = vec![0xff];

    let create_params = CreateExternalParams(initcode.clone());

    let evm_params = EvmConstructorParams { creator: caller_eth_addr, initcode: initcode.into() };

    let params = Exec4Params {
        code_cid: *EVM_ACTOR_CODE_ID,
        constructor_params: RawBytes::serialize(evm_params).unwrap(),
        subaddress: target_eth_addr.0[..].to_owned().into(),
    };

    let send_return = Exec4Return {
        id_address: Address::new_id(111),
        robust_address: Address::new_actor(&[0xde, 0xad, 0xbe, 0xef]),
    };
    let send_return_ser = IpldBlock::serialize_cbor(&send_return).unwrap();

    rt.expect_send(
        INIT_ACTOR_ADDR,
        EXEC4_METHOD,
        IpldBlock::serialize_cbor(&params).unwrap(),
        TokenAmount::from_atto(0),
        send_return_ser,
        ExitCode::OK,
    );

    rt.expect_validate_caller_addr(vec![caller_id_addr]);
    let result = rt
        .call::<eam::EamActor>(
            eam::Method::CreateExternal as u64,
            IpldBlock::serialize_cbor(&create_params).unwrap(),
        )
        .unwrap()
        .unwrap()
        .deserialize::<Return>()
        .unwrap();

    let expected_return = Return {
        actor_id: target_id_addr.id().unwrap(),
        robust_address: Some(send_return.robust_address),
        eth_address: target_eth_addr,
    };

    assert_eq!(result, expected_return);
    rt.verify();
}

#[test]
fn call_resurrect() {
    let mut rt = construct_and_verify();

    let caller_id_addr = Address::new_id(110);
    let caller_eth_addr =
        eam::EthAddress(hex_literal::hex!("CAFEB0BA00000000000000000000000000000000"));
    let caller_f4_eth_addr = Address::new_delegated(10, &caller_eth_addr.0).unwrap();
    rt.add_delegated_address(caller_id_addr, caller_f4_eth_addr);

    rt.set_caller(*EVM_ACTOR_CODE_ID, caller_id_addr);

    let target_id_addr = Address::new_id(111);
    let target_eth_addr = compute_address_create(&rt, &caller_eth_addr, 0);
    let target_f4_eth_addr = Address::new_delegated(10, &target_eth_addr.0).unwrap();
    rt.add_delegated_address(target_id_addr, target_f4_eth_addr);
    rt.set_address_actor_type(target_id_addr, *EVM_ACTOR_CODE_ID);

    rt.expect_validate_caller_type(vec![Type::EVM]);

    let initcode = vec![0xff];

    let create_params = CreateParams { initcode: initcode.clone(), nonce: 0 };

    let params = EvmConstructorParams { creator: caller_eth_addr, initcode: initcode.into() };

    rt.expect_send(
        target_id_addr,
        RESURRECT_METHOD,
        IpldBlock::serialize_cbor(&params).unwrap(),
        TokenAmount::from_atto(0),
        None,
        ExitCode::OK,
    );

    let result = rt
        .call::<eam::EamActor>(
            eam::Method::Create as u64,
            IpldBlock::serialize_cbor(&create_params).unwrap(),
        )
        .unwrap()
        .unwrap()
        .deserialize::<Return>()
        .unwrap();

    let expected_return =
        Return { actor_id: 111, robust_address: None, eth_address: target_eth_addr };

    assert_eq!(result, expected_return);
    rt.verify();
}

#[test]
fn call_create2() {
    let mut rt = construct_and_verify();

    let id_addr = Address::new_id(110);
    let eth_addr = eam::EthAddress(hex_literal::hex!("CAFEB0BA00000000000000000000000000000000"));
    let f4_eth_addr = Address::new_delegated(10, &eth_addr.0).unwrap();
    rt.add_delegated_address(id_addr, f4_eth_addr);

    rt.set_caller(*EVM_ACTOR_CODE_ID, id_addr);
    rt.expect_validate_caller_type(vec![Type::EVM]);

    let initcode = vec![0xff];

    let create2_params = Create2Params { initcode: initcode.clone(), salt: [0; 32] };

    let evm_params = EvmConstructorParams { creator: eth_addr, initcode: initcode.clone().into() };

    let inithash = rt.hash(fvm_shared::crypto::hash::SupportedHashes::Keccak256, &initcode);
    let mut subaddress = rt.hash(
        fvm_shared::crypto::hash::SupportedHashes::Keccak256,
        &[&[0xff], &eth_addr.0[..], &create2_params.salt, &inithash].concat(),
    );
    subaddress.drain(..12);

    let params = Exec4Params {
        code_cid: *EVM_ACTOR_CODE_ID,
        constructor_params: RawBytes::serialize(evm_params).unwrap(),
        subaddress: subaddress.clone().into(),
    };

    let send_return = IpldBlock::serialize_cbor(&Exec4Return {
        id_address: Address::new_id(111),
        robust_address: Address::new_id(0), // not a robust address but im hacking here and nobody checks
    })
    .unwrap();

    rt.expect_send(
        INIT_ACTOR_ADDR,
        EXEC4_METHOD,
        IpldBlock::serialize_cbor(&params).unwrap(),
        TokenAmount::from_atto(0),
        send_return,
        ExitCode::OK,
    );

    let result = rt
        .call::<eam::EamActor>(
            eam::Method::Create2 as u64,
            IpldBlock::serialize_cbor(&create2_params).unwrap(),
        )
        .unwrap()
        .unwrap()
        .deserialize::<Return>()
        .unwrap();

    let expected_return = Return {
        actor_id: 111,
        robust_address: Some(Address::new_id(0)),
        eth_address: EthAddress(subaddress.try_into().unwrap()),
    };

    assert_eq!(result, expected_return);
    rt.verify();
}

pub fn construct_and_verify() -> MockRuntime {
    let mut rt = MockRuntime { receiver: Address::new_id(10), ..Default::default() };

    // construct EAM singleton actor
    rt.set_caller(*SYSTEM_ACTOR_CODE_ID, SYSTEM_ACTOR_ADDR);

    rt.expect_validate_caller_addr(vec![SYSTEM_ACTOR_ADDR]);

    let result = rt.call::<eam::EamActor>(eam::Method::Constructor as u64, None).unwrap();
    expect_empty(result);
    rt.verify();
    rt.reset();

    rt
}
