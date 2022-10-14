use eam::ext::init::{Exec4Params, Exec4Return, EXEC4_METHOD};
use eam::{
    Create2Params, CreateParams, EamReturn, EthAddress, EvmConstructorParams, RlpCreateAddress,
};
use fil_actor_eam as eam;
use fil_actors_runtime::runtime::builtins::Type;
use fil_actors_runtime::runtime::Primitives;
use fil_actors_runtime::test_utils::{
    expect_empty, MockRuntime, EVM_ACTOR_CODE_ID, INIT_ACTOR_CODE_ID,
};
use fil_actors_runtime::INIT_ACTOR_ADDR;
use fvm_ipld_encoding::RawBytes;
use fvm_shared::address::Address;
use fvm_shared::econ::TokenAmount;
use fvm_shared::error::ExitCode;
use rlp::Encodable;

#[test]
fn call_create() {
    let mut rt = construct_and_verify();

    let id_addr = Address::new_id(110);
    let eth_addr = eam::EthAddress(hex_literal::hex!("CAFEB0BA00000000000000000000000000000000"));
    let f4_eth_addr = Address::new_delegated(10, &eth_addr.0).unwrap();
    rt.add_delegated_address(id_addr, f4_eth_addr);

    rt.set_caller(*EVM_ACTOR_CODE_ID, id_addr);
    rt.expect_validate_caller_type(vec![Type::EVM]);

    let initcode = vec![0xff];

    let create_params = CreateParams { initcode: initcode.clone().into(), nonce: 0 };

    let evm_params = EvmConstructorParams { creator: eth_addr, initcode: initcode.into() };

    let rlp_params = RlpCreateAddress { address: eth_addr, nonce: 0 };
    let mut subaddress =
        rt.hash(fvm_shared::crypto::hash::SupportedHashes::Keccak256, &rlp_params.rlp_bytes());
    subaddress.drain(..12);

    let params = Exec4Params {
        code_cid: *EVM_ACTOR_CODE_ID,
        constructor_params: RawBytes::serialize(evm_params.clone()).unwrap(),
        subaddress: subaddress.clone().into(),
    };

    let send_return = RawBytes::serialize(Exec4Return {
        id_address: Address::new_id(111),
        robust_address: Address::new_id(0), // not a robust address but im hacking here and nobody checks
    })
    .unwrap();

    rt.expect_send(
        INIT_ACTOR_ADDR,
        EXEC4_METHOD,
        RawBytes::serialize(params).unwrap(),
        TokenAmount::from_atto(0),
        send_return,
        ExitCode::OK,
    );

    let result = rt
        .call::<eam::EamActor>(
            eam::Method::Create as u64,
            &RawBytes::serialize(create_params).unwrap(),
        )
        .unwrap()
        .deserialize::<EamReturn>()
        .unwrap();

    let expected_return = EamReturn {
        actor_id: 111,
        robust_address: Address::new_id(0),
        eth_address: EthAddress(subaddress.try_into().unwrap()),
    };

    assert_eq!(result, expected_return)
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

    let create2_params = Create2Params { initcode: initcode.clone().into(), salt: [0; 32] };

    let evm_params = EvmConstructorParams { creator: eth_addr, initcode: initcode.clone().into() };

    let inithash = rt.hash(fvm_shared::crypto::hash::SupportedHashes::Keccak256, &initcode);
    let mut subaddress = rt.hash(
        fvm_shared::crypto::hash::SupportedHashes::Keccak256,
        &[&[0xff], &eth_addr.0[..], &create2_params.salt, &inithash].concat(),
    );
    subaddress.drain(..12);

    let params = Exec4Params {
        code_cid: *EVM_ACTOR_CODE_ID,
        constructor_params: RawBytes::serialize(evm_params.clone()).unwrap(),
        subaddress: subaddress.clone().into(),
    };

    let send_return = RawBytes::serialize(Exec4Return {
        id_address: Address::new_id(111),
        robust_address: Address::new_id(0), // not a robust address but im hacking here and nobody checks
    })
    .unwrap();

    rt.expect_send(
        INIT_ACTOR_ADDR,
        EXEC4_METHOD,
        RawBytes::serialize(params).unwrap(),
        TokenAmount::from_atto(0),
        send_return,
        ExitCode::OK,
    );

    let result = rt
        .call::<eam::EamActor>(
            eam::Method::Create2 as u64,
            &RawBytes::serialize(create2_params).unwrap(),
        )
        .unwrap()
        .deserialize::<EamReturn>()
        .unwrap();

    let expected_return = EamReturn {
        actor_id: 111,
        robust_address: Address::new_id(0),
        eth_address: EthAddress(subaddress.try_into().unwrap()),
    };

    assert_eq!(result, expected_return)
}

pub fn construct_and_verify() -> MockRuntime {
    let mut rt = MockRuntime::default();

    rt.receiver = Address::new_id(10);

    // construct EAM singleton actor
    rt.set_caller(*INIT_ACTOR_CODE_ID, INIT_ACTOR_ADDR);

    rt.expect_validate_caller_type(vec![Type::Init]);

    let result =
        rt.call::<eam::EamActor>(eam::Method::Constructor as u64, &RawBytes::default()).unwrap();
    expect_empty(result);
    rt.verify();
    rt.reset();

    rt
}
