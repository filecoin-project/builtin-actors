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
    MockRuntime::default();

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

#[allow(dead_code)]
pub fn methodnum_contract() -> Vec<u8> {
    // a simple contract that just returns the invocation methodnum
    let init = "";
    let body = r#"
@methodnum
push1 0x00
mstore

push1 0x20
push1 0x00
return
"#;

    asm::new_contract("methodnum", init, body).unwrap()
}

#[test]
fn test_methodnum() {
    let contract = methodnum_contract();

    let mut rt = util::construct_and_verify(contract);

    // invoke the contract
    rt.expect_validate_caller_any();

    let result = rt.call::<evm::EvmContractActor>(0x42, &RawBytes::default()).unwrap();
    assert_eq!(U256::from_big_endian(&result), U256::from(0x42));
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
