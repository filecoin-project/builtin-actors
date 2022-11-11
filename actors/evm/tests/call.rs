mod asm;

use evm::interpreter::address::EthAddress;
use evm::interpreter::U256;
use fil_actor_evm as evm;
use fil_actors_runtime::test_utils::*;
use fvm_ipld_encoding::{BytesSer, RawBytes};
use fvm_shared::address::Address as FILAddress;
use fvm_shared::bigint::Zero;
use fvm_shared::econ::TokenAmount;
use fvm_shared::error::ExitCode;
use fvm_shared::METHOD_SEND;

mod util;

#[allow(dead_code)]
pub fn call_proxy_contract() -> Vec<u8> {
    let init = "";
    let body = r#"
# this contract takes an address and the call payload and proxies a call to that address
# get call payload size
push1 0x20
calldatasize
sub
# store payload to mem 0x00
push1 0x20
push1 0x00
calldatacopy

# prepare the proxy call
# output offset and size -- 0 in this case, we use returndata
push2 0x00
push1 0x00
# input offset and size
push1 0x20
calldatasize
sub
push1 0x00
# value
push1 0x00
# dest address
push1 0x00
calldataload
# gas
push1 0x00
# do the call
call

# return result through
returndatasize
push1 0x00
push1 0x00
returndatacopy
returndatasize
push1 0x00
return
"#;

    asm::new_contract("call-proxy", init, body).unwrap()
}

#[test]
fn test_call() {
    let contract = call_proxy_contract();

    // construct the proxy
    let mut rt = util::construct_and_verify(contract);

    // create a mock target and proxy a call through the proxy
    let target_id = 0x100;
    let target = FILAddress::new_id(target_id);
    let evm_target = EthAddress(hex_literal::hex!("deadbeefdeadbeefdeadbeefdeadbeefdeadbeef"));
    let f4_target: FILAddress = evm_target.try_into().unwrap();
    rt.actor_code_cids.insert(target, *EVM_ACTOR_CODE_ID);
    rt.add_delegated_address(target, f4_target);

    let evm_target_word = evm_target.as_evm_word();

    // dest + method 0 with no data
    let mut contract_params = vec![0u8; 36];
    evm_target_word.to_big_endian(&mut contract_params[..32]);

    let proxy_call_contract_params = vec![0u8; 4];
    let proxy_call_input_data = RawBytes::serialize(BytesSer(&proxy_call_contract_params))
        .expect("failed to serialize input data");

    // expected return data
    let mut return_data = vec![0u8; 32];
    return_data[31] = 0x42;

    rt.expect_send(
        f4_target,
        evm::Method::InvokeContract as u64,
        proxy_call_input_data,
        TokenAmount::zero(),
        RawBytes::serialize(BytesSer(&return_data)).expect("failed to serialize return data"),
        ExitCode::OK,
    );

    let result = util::invoke_contract(&mut rt, &contract_params);
    assert_eq!(U256::from_big_endian(&result), U256::from(0x42));
}

// Test that a zero-value call to an actor that doesn't exist doesn't actually create the actor.
#[test]
fn test_empty_call_no_side_effects() {
    let contract = call_proxy_contract();

    // construct the proxy
    let mut rt = util::construct_and_verify(contract);

    // create a mock target and proxy a call through the proxy
    let evm_target = EthAddress(hex_literal::hex!("deadbeefdeadbeefdeadbeefdeadbeefdeadbeef"));

    let evm_target_word = evm_target.as_evm_word();

    // dest + method 0 with no data
    let mut contract_params = vec![0u8; 36];
    evm_target_word.to_big_endian(&mut contract_params[..32]);

    // expected return data
    let mut return_data = vec![0u8; 32];
    return_data[31] = 0x42;

    let result = util::invoke_contract(&mut rt, &contract_params);
    assert_eq!(U256::from_big_endian(&result), U256::from(0));
    // Expect no calls
    rt.verify();
}

// Make sure we do bare sends when calling accounts/embryo, and make sure it works.
#[test]
fn test_call_convert_to_send() {
    let contract = call_proxy_contract();

    for code in [*ACCOUNT_ACTOR_CODE_ID, *EMBRYO_ACTOR_CODE_ID] {
        // construct the proxy
        let mut rt = util::construct_and_verify(contract.clone());

        // create a mock actor and proxy a call through the proxy
        let target_id = 0x100;
        let target = FILAddress::new_id(target_id);
        rt.actor_code_cids.insert(target, code);

        let evm_target_word = EthAddress::from_id(target_id).as_evm_word();

        // dest + method 0 with no data
        let mut contract_params = vec![0u8; 36];
        evm_target_word.to_big_endian(&mut contract_params[..32]);

        let proxy_call_contract_params = vec![0u8; 4];
        let proxy_call_input_data = RawBytes::serialize(BytesSer(&proxy_call_contract_params))
            .expect("failed to serialize input data");

        // expected return data
        let mut return_data = vec![0u8; 32];
        return_data[31] = 0x42;

        rt.expect_send(
            target,
            METHOD_SEND,
            proxy_call_input_data,
            TokenAmount::zero(),
            RawBytes::serialize(BytesSer(&return_data)).expect("failed to serialize return data"),
            ExitCode::OK,
        );

        let result = util::invoke_contract(&mut rt, &contract_params);
        assert_eq!(U256::from_big_endian(&result), U256::from(0x42));
        rt.verify();
    }
}

#[allow(dead_code)]
pub fn filecoin_fallback_contract() -> Vec<u8> {
    hex::decode(include_str!("contracts/FilecoinFallback.hex")).unwrap()
}

#[test]
fn test_reserved_method() {
    let contract = filecoin_fallback_contract();
    let mut rt = util::construct_and_verify(contract);

    // invoke the contract
    rt.expect_validate_caller_any();

    let code =
        rt.call::<evm::EvmContractActor>(0x42, &RawBytes::default()).unwrap_err().exit_code();
    assert_eq!(ExitCode::USR_UNHANDLED_MESSAGE, code);
}

#[test]
fn test_native_call() {
    let contract = filecoin_fallback_contract();
    let mut rt = util::construct_and_verify(contract);

    // invoke the contract
    rt.expect_validate_caller_any();

    let result = rt.call::<evm::EvmContractActor>(1024, &RawBytes::default()).unwrap();
    assert_eq!(U256::from_big_endian(&result), U256::from(1024));
}

#[allow(dead_code)]
pub fn callactor_proxy_contract() -> Vec<u8> {
    let init = "";
    let body = r#"
# this contract takes an address, method and the call payload and proxies a CALLACTOR call
# to that address
# get call payload size
push1 0x40
calldatasize
sub
# store payload to mem 0x00
push1 0x40
push1 0x00
calldatacopy

# prepare the proxy call
# input offset and size
push1 0x40
calldatasize
sub
push1 0x00
# method
push1 0x20
calldataload
# value
push1 0x00
# dest address
push1 0x00
calldataload
# gas
push1 0x00
# do the call
@callactor

# return result through
returndatasize
push1 0x00
push1 0x00
returndatacopy
returndatasize
push1 0x00
return
"#;

    asm::new_contract("callactor-proxy", init, body).unwrap()
}

#[test]
fn test_callactor() {
    let contract = callactor_proxy_contract();

    // construct the proxy
    let mut rt = util::construct_and_verify(contract);

    // create a mock target and proxy a call through the proxy
    let target_id = 0x100;
    let target = FILAddress::new_id(target_id);
    rt.actor_code_cids.insert(target, *EVM_ACTOR_CODE_ID);

    let evm_target = EthAddress::from_id(target_id);
    let evm_target_word = evm_target.as_evm_word();

    // dest + method 0x42 with no data
    let mut contract_params = vec![0u8; 64];
    evm_target_word.to_big_endian(&mut contract_params[..32]);
    contract_params[63] = 0x42;

    let proxy_call_input_data = RawBytes::default();

    // expected return data
    let mut return_data = vec![0u8; 32];
    return_data[31] = 0x42;

    rt.expect_send(
        target,
        0x42,
        proxy_call_input_data,
        TokenAmount::zero(),
        RawBytes::from(return_data),
        ExitCode::OK,
    );
    let result = util::invoke_contract(&mut rt, &contract_params);

    assert_eq!(U256::from_big_endian(&result), U256::from(0x42));
}
