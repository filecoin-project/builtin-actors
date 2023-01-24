mod asm;

use std::fmt::Debug;
use std::sync::Arc;

use ethers::abi::Detokenize;
use ethers::prelude::builders::ContractCall;
use ethers::prelude::*;
use ethers::providers::{MockProvider, Provider};
use ethers::types::Bytes;
use evm::interpreter::address::EthAddress;
use evm::interpreter::U256;
use evm::{Method, EVM_CONTRACT_REVERTED};
use fil_actor_evm as evm;
use fil_actors_runtime::{test_utils::*, EAM_ACTOR_ID, INIT_ACTOR_ADDR};
use fvm_ipld_encoding::ipld_block::IpldBlock;
use fvm_ipld_encoding::{BytesDe, BytesSer, CBOR, IPLD_RAW};
use fvm_shared::address::Address as FILAddress;
use fvm_shared::address::Address;
use fvm_shared::bigint::Zero;
use fvm_shared::econ::TokenAmount;
use fvm_shared::error::{ExitCode, ErrorNumber};
use fvm_shared::sys::SendFlags;
use fvm_shared::{ActorID, MethodNum, METHOD_SEND};
use once_cell::sync::Lazy;

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
push4 0xffffffff
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

#[allow(dead_code)]
pub fn call_proxy_transfer_contract() -> Vec<u8> {
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
push1 0x42
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

    asm::new_contract("call-proxy-transfer", init, body).unwrap()
}

#[allow(dead_code)]
pub fn call_proxy_gas2300_contract() -> Vec<u8> {
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
%push(2300)
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

    asm::new_contract("call-proxy-gas2300", init, body).unwrap()
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
    let proxy_call_input_data = make_raw_params(proxy_call_contract_params);

    // expected return data
    let mut return_data = vec![0u8; 32];
    return_data[31] = 0x42;

    rt.expect_gas_available(10_000_000_000u64);
    rt.expect_send_generalized(
        f4_target,
        evm::Method::InvokeContract as u64,
        proxy_call_input_data,
        TokenAmount::zero(),
        Some(0xffffffff),
        SendFlags::empty(),
        IpldBlock::serialize_cbor(&BytesSer(&return_data))
            .expect("failed to serialize return data"),
        ExitCode::OK,
        None,
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

// Make sure we do bare sends when calling accounts/placeholder, and make sure it works.
#[test]
fn test_call_convert_to_send() {
    let contract = call_proxy_contract();

    for code in [*ACCOUNT_ACTOR_CODE_ID, *PLACEHOLDER_ACTOR_CODE_ID] {
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
        let proxy_call_input_data = make_raw_params(proxy_call_contract_params);

        // expected return data
        let mut return_data = vec![0u8; 32];
        return_data[31] = 0x42;

        rt.expect_send_generalized(
            target,
            METHOD_SEND,
            proxy_call_input_data,
            TokenAmount::zero(),
            None,
            SendFlags::empty(),
            IpldBlock::serialize_cbor(&BytesSer(&return_data))
                .expect("failed to serialize return data"),
            ExitCode::OK,
            None,
        );

        let result = util::invoke_contract(&mut rt, &contract_params);
        assert_eq!(U256::from_big_endian(&result), U256::from(0x42));
        rt.verify();
    }
}

// Make sure we do bare sends when calling with 0 gas and value
#[test]
fn test_call_convert_to_send2() {
    let contract = call_proxy_transfer_contract();

    // construct the proxy
    let mut rt = util::construct_and_verify(contract);

    // create a mock actor and proxy a call through the proxy
    let target_id = 0x100;
    let target = FILAddress::new_id(target_id);
    rt.actor_code_cids.insert(target, *EVM_ACTOR_CODE_ID);

    let evm_target_word = EthAddress::from_id(target_id).as_evm_word();

    // dest with no data
    let mut contract_params = vec![0u8; 32];
    evm_target_word.to_big_endian(&mut contract_params);

    // we don't expected return data
    let return_data = vec![];

    rt.expect_send(
        target,
        METHOD_SEND,
        None,
        TokenAmount::from_atto(0x42),
        IpldBlock::serialize_cbor(&BytesSer(&return_data))
            .expect("failed to serialize return data"),
        ExitCode::OK,
    );

    let result = util::invoke_contract(&mut rt, &contract_params);
    assert_eq!(U256::from_big_endian(&result), U256::from(0));
    rt.verify();
}

// Make sure we do bare sends when calling with 2300 gas and no value
#[test]
fn test_call_convert_to_send3() {
    let contract = call_proxy_gas2300_contract();

    // construct the proxy
    let mut rt = util::construct_and_verify(contract);

    // create a mock actor and proxy a call through the proxy
    let target_id = 0x100;
    let target = FILAddress::new_id(target_id);
    rt.actor_code_cids.insert(target, *EVM_ACTOR_CODE_ID);

    let evm_target_word = EthAddress::from_id(target_id).as_evm_word();

    // dest with no data
    let mut contract_params = vec![0u8; 32];
    evm_target_word.to_big_endian(&mut contract_params);

    // we don't expected return data
    let return_data = vec![];

    rt.expect_send(
        target,
        METHOD_SEND,
        None,
        TokenAmount::zero(),
        IpldBlock::serialize_cbor(&BytesSer(&return_data))
            .expect("failed to serialize return data"),
        ExitCode::OK,
    );

    let result = util::invoke_contract(&mut rt, &contract_params);
    assert_eq!(U256::from_big_endian(&result), U256::from(0));
    rt.verify();
}

#[test]
pub fn test_call_output_region() {
    let init = "";
    let body = r#"
# this contract truncates return from send to output length

# prepare the proxy call
push1 0x00 
calldataload # size from first word
push1 0x00 # offset

# input offset and size
push1 0x00
push1 0x00

# value
push1 0x00

# dest address
push1 0x20
calldataload # address from second word

# gas
push1 0x00

# do the call
call

push1 0x40
calldataload # return size from third word
push1 0x00 # offset
return
"#;

    let contract = asm::new_contract("call-output-region", init, body).unwrap();
    let mut rt = util::construct_and_verify(contract);

    let address = EthAddress(util::CONTRACT_ADDRESS);

    // large set of data
    let large_ret = IpldBlock { codec: CBOR, data: vec![0xff; 2048] };

    let cases = [(32, 64), (64, 64), (1024, 1025)];
    for (output_size, return_size) in cases {
        rt.expect_send_generalized(
            (&address).try_into().unwrap(),
            Method::InvokeContract as u64,
            None,
            TokenAmount::zero(),
            Some(0),
            SendFlags::empty(),
            Some(large_ret.clone()),
            ExitCode::OK,
            None,
        );

        rt.expect_gas_available(10_000_000_000);

        let out = util::invoke_contract(
            &mut rt,
            &[
                U256::from(output_size).to_bytes().to_vec(),
                address.as_evm_word().to_bytes().to_vec(),
                U256::from(return_size).to_bytes().to_vec(),
            ]
            .concat(),
        );
        let mut expected = vec![0xff; output_size];
        expected.extend_from_slice(&vec![0u8; return_size - output_size]);

        rt.verify();
        assert_eq!(
            expected,
            out,
            "expect: {}\n   got: {}",
            hex::encode(&expected),
            hex::encode(&out)
        );
        rt.reset();
    }
}

#[allow(dead_code)]
pub fn filecoin_fallback_contract() -> Vec<u8> {
    hex::decode(include_str!("contracts/FilecoinFallback.hex")).unwrap()
}

#[allow(dead_code)]
pub fn filecoin_call_actor_contract() -> Vec<u8> {
    hex::decode(include_str!("contracts/CallActorPrecompile.hex")).unwrap()
}

#[test]
fn test_reserved_method() {
    let contract = filecoin_fallback_contract();
    let mut rt = util::construct_and_verify(contract);

    let code = rt.call::<evm::EvmContractActor>(0x42, None).unwrap_err().exit_code();
    assert_eq!(ExitCode::USR_UNHANDLED_MESSAGE, code);
}

#[test]
fn test_native_call() {
    let contract = filecoin_fallback_contract();
    let mut rt = util::construct_and_verify(contract);

    rt.expect_validate_caller_any();
    let result = rt.call::<evm::EvmContractActor>(1024, None).unwrap();
    assert_eq!(result, None);

    rt.expect_validate_caller_any();
    let result = rt.call::<evm::EvmContractActor>(1025, None).unwrap();
    assert_eq!(result, Some(IpldBlock { codec: CBOR, data: "foobar".into() }));

    rt.expect_validate_caller_any();
    let err = rt.call::<evm::EvmContractActor>(1026, None).unwrap_err();
    assert_eq!(err.exit_code().value(), 42);
    assert!(err.data().is_empty());

    rt.expect_validate_caller_any();
    let err = rt.call::<evm::EvmContractActor>(1027, None).unwrap_err();
    assert_eq!(err.exit_code().value(), 42);
    assert_eq!(err.data(), &b"foobar"[..]);
}

#[test]
fn test_callactor_success() {
    // Should work if the called actor succeeds.
    test_callactor_inner(2048, ExitCode::OK, true)
}

#[test]
fn test_callactor_revert() {
    // Should propagate the return value if the called actor fails.
    test_callactor_inner(2048, EVM_CONTRACT_REVERTED, true)
}

// Much taken from tests/env.rs
abigen!(CallActorPrecompile, "./tests/contracts/CallActorPrecompile.abi");

const OWNER_ID: ActorID = 1001;
const _OWNER: Address = Address::new_id(OWNER_ID);
static CONTRACT: Lazy<CallActorPrecompile<Provider<MockProvider>>> = Lazy::new(|| {
    // The owner of the contract is expected to be the 160 bit hash used on Ethereum.
    // We're not going to use it during the tests.
    let address = EthAddress::from_id(OWNER_ID);
    let address = ethers::core::types::Address::from_slice(address.as_ref());
    // A dummy client that we don't intend to use to call the contract or send transactions.
    let (client, _mock) = Provider::mocked();
    CallActorPrecompile::new(address, Arc::new(client))
});

pub type TestContractCall<R> = ContractCall<Provider<MockProvider>, R>;

#[test]
fn test_callactor_restrict() {
    // Should propagate the return value if the called actor fails.
    test_callactor_inner(2, EVM_CONTRACT_REVERTED, false)
}

fn test_callactor_inner(method_num: MethodNum, exit_code: ExitCode, valid_call_input: bool) {
    let contract = {
        let (init, body) = util::PrecompileTest::test_runner_assembly();
        asm::new_contract("call_actor-precompile-test", &init, &body).unwrap()
    };

    const CALLACTOR_NUM_PARAMS: usize = 8;

    // construct the proxy
    let mut rt = util::construct_and_verify(contract);

    // create a mock target and proxy a call through the proxy
    let target_id = 0x100;
    let target = FILAddress::new_id(target_id);
    rt.actor_code_cids.insert(target, *EVM_ACTOR_CODE_ID);

    // dest + method with no data
    let mut contract_params = Vec::new();

    let method = U256::from(method_num);
    let value = U256::from(0);
    let send_flags = SendFlags::default();
    let codec = U256::from(0);

    let proxy_call_input_data = vec![];
    let data_size = U256::from(proxy_call_input_data.len());

    let target_bytes = target.to_bytes();
    let target_size = U256::from(target_bytes.len());

    let data_off = U256::from(6 * 32);
    let target_off = data_off + 32 + data_size;

    // a bit messy but a "test" for CallActorParams
    let params: Vec<u8> = CallActorParams {
        method: U256::from(method_num),
        value: U256::from(0),
        flags: U256::from(0),
        codec: U256::from(0),
        param_offset: data_off,
        addr_offset: target_off,
        param_len: data_size,
        params: Some(proxy_call_input_data.clone()),
        addr_len: target_size,
        addr: target_bytes.clone(),
    }
    .into();

    contract_params.extend_from_slice(&method.to_bytes());
    contract_params.extend_from_slice(&value.to_bytes());
    contract_params.extend_from_slice(&U256::from(send_flags.bits()).to_bytes());
    contract_params.extend_from_slice(&codec.to_bytes());
    contract_params.extend_from_slice(&data_off.to_bytes());
    contract_params.extend_from_slice(&target_off.to_bytes());
    contract_params.extend_from_slice(&data_size.to_bytes());
    contract_params.extend_from_slice(&proxy_call_input_data);
    contract_params.extend_from_slice(&target_size.to_bytes());
    contract_params.extend_from_slice(&target_bytes);

    assert_eq!(
        params,
        contract_params,
        "{}\n{}",
        hex::encode(&params),
        hex::encode(&contract_params)
    );

    assert_eq!(
        32 * CALLACTOR_NUM_PARAMS + target_bytes.len() + proxy_call_input_data.len(),
        contract_params.len(),
        "unexpected input length"
    );

    // expected return data
    // Test with a codec _other_ than CBOR/DAG_CBOR, to make sure we are actually passing the returned codec
    let some_codec = 0x42;
    let data = vec![0xde, 0xad, 0xbe, 0xef];
    let send_return = IpldBlock { codec: some_codec, data };

    if valid_call_input {
        // We only get to the send_generalized if the call params were valid
        rt.expect_send_generalized(
            target,
            method_num,
            make_raw_params(proxy_call_input_data),
            TokenAmount::zero(),
            Some(0),
            send_flags,
            Some(send_return.clone()),
            exit_code,
            None,
        );
    }

    // output bytes are padded to nearest 32 byte
    let mut v = vec![0; 32];
    v[..4].copy_from_slice(&send_return.data);

    let expect = CallActorReturn {
        send_exit_code,
        codec: send_return.codec,
        data_offset: 96,
        data_size: send_return.data.len() as u32,
        data: v,
    };

    let (expected_exit, expected_out) = if valid_call_input {
        (util::PrecompileExit::Success, expect.into())
    } else {
        (util::PrecompileExit::Reverted, vec![])
    };

    let test = util::PrecompileTest {
        expected_exit_code: expected_exit,
        precompile_address: util::NativePrecompile::CallActor.eth_address(),
        output_size: 32,
        gas_avaliable: 10_000_000_000u64,
        expected_return: expected_out,
        call_op: util::PrecompileCallOpcode::DelegateCall,
        input: contract_params,
    };

    // invoke
    test.run_test(&mut rt);
}

#[test]
fn call_actor_weird_offset() {
    let contract = {
        let (init, body) = util::PrecompileTest::test_runner_assembly();
        asm::new_contract("call_actor-precompile-test", &init, &body).unwrap()
    };
    let mut rt = util::construct_and_verify(contract);

    let addr = Address::new_delegated(1234, b"foobarboxy").unwrap();
    let addr_bytes = addr.to_bytes();
    let params = CallActorParams {
        method: U256::from(0),
        value: U256::from(0),
        flags: U256::from(0),
        codec: U256::from(0),
        param_offset: U256::from(200),
        addr_offset: U256::from(300),
        param_len: U256::from(0),
        params: None,
        addr_len: U256::from(addr_bytes.len()),
        addr: addr_bytes,
    };

    let input: Vec<u8> = params.into();

    let mut test = util::PrecompileTest {
        expected_exit_code: util::PrecompileExit::Success,
        precompile_address: util::NativePrecompile::CallActor.eth_address(),
        output_size: 32,
        gas_avaliable: 10_000_000_000u64,
        expected_return: vec![],
        call_op: util::PrecompileCallOpcode::DelegateCall,
        input,
    };

    rt.expect_send_generalized(
        addr,
        0,
        None,
        TokenAmount::zero(),
        Some(0),
        SendFlags::empty(),
        None,
        ExitCode::OK,
        None,
    );

    let precompile_return = CallActorReturn::default();

    test.run_test_expecting(&mut rt, precompile_return, util::PrecompileExit::Success);
}

#[test]
fn call_actor_overlapping() {
    let contract = {
        let (init, body) = util::PrecompileTest::test_runner_assembly();
        asm::new_contract("call_actor-precompile-test", &init, &body).unwrap()
    };
    let mut rt = util::construct_and_verify(contract);
    let addr = Address::new_delegated(1234, b"foobarboxy").unwrap();

    let mut call_params = CallActorParams::default();

    // not valid CBOR, but params should parse fine in precompile
    let addr_bytes = addr.to_bytes();
    call_params.codec(U256::from(CBOR));

    call_params.param_offset = U256::from(CallActorParams::FIRST_DYNAMIC_OFFSET);
    call_params.param_len = U256::from(addr_bytes.len());
    call_params.params = None;

    call_params.addr_offset = U256::from(CallActorParams::FIRST_DYNAMIC_OFFSET);
    call_params.addr_len = U256::from(addr_bytes.len());
    call_params.addr = addr_bytes.clone();

    let mut test = util::PrecompileTest {
        precompile_address: util::NativePrecompile::CallActor.eth_address(),
        output_size: 32,
        gas_avaliable: 10_000_000_000u64,
        call_op: util::PrecompileCallOpcode::DelegateCall,
        // overwritten in tests
        expected_return: vec![],
        expected_exit_code: util::PrecompileExit::Success,
        input: call_params.clone().into(),
    };

    rt.expect_send_generalized(
        addr,
        0,
        Some(IpldBlock { codec: CBOR, data: addr_bytes }),
        TokenAmount::zero(),
        Some(0),
        SendFlags::empty(),
        None,
        ExitCode::OK,
        None,
    );

    test.input = call_params.into();
    test.run_test_expecting(&mut rt, CallActorReturn::default(), util::PrecompileExit::Success);
}

#[test]
fn call_actor_id_with_full_address() {
    let contract = {
        let (init, body) = util::PrecompileTest::test_runner_assembly();
        asm::new_contract("call_actor-precompile-test", &init, &body).unwrap()
    };
    let mut rt = util::construct_and_verify(contract);
    let addr = Address::new_delegated(1234, b"foobarboxy").unwrap();
    let actual_id_addr = 1234;

    let mut call_params = CallActorParams::default();
    // garbage bytes
    call_params.set_addr(CallActorParams::EMPTY_PARAM_ADDR_OFFSET, addr.to_bytes());
    // id address
    call_params.addr_offset = U256::from(actual_id_addr);

    let mut test = util::PrecompileTest {
        precompile_address: util::NativePrecompile::CallActorId.eth_address(),
        output_size: 32,
        gas_avaliable: 10_000_000_000u64,
        call_op: util::PrecompileCallOpcode::DelegateCall,
        // overwritten in tests
        expected_return: vec![],
        expected_exit_code: util::PrecompileExit::Success,
        input: call_params.clone().into(),
    };

    rt.expect_send_generalized(
        Address::new_id(actual_id_addr),
        0,
        None,
        TokenAmount::zero(),
        Some(0),
        SendFlags::empty(),
        None,
        ExitCode::OK,
        None,
    );

    test.input = call_params.into();
    test.run_test_expecting(&mut rt, CallActorReturn::default(), util::PrecompileExit::Success);
}


#[test]
fn call_actor_syscall_error() {
    let contract = {
        let (init, body) = util::PrecompileTest::test_runner_assembly();
        asm::new_contract("call_actor-precompile-test", &init, &body).unwrap()
    };
    let mut rt = util::construct_and_verify(contract);
    let addr = Address::new_delegated(1234, b"foobarboxy").unwrap();

    let call_params = CallActorParams::default();

    let mut test = util::PrecompileTest {
        precompile_address: util::NativePrecompile::CallActor.eth_address(),
        output_size: 32,
        gas_avaliable: 10_000_000_000u64,
        call_op: util::PrecompileCallOpcode::DelegateCall,
        // overwritten in tests
        expected_return: vec![],
        expected_exit_code: util::PrecompileExit::Success,
        input: call_params.clone().into(),
    };

    let syscall_exit = ErrorNumber::NotFound;

    let mut expect = CallActorReturn::default();
    expect.send_exit_code = U256::from(syscall_exit as u32).i256_neg();

    rt.expect_send_generalized(
        addr,
        0,
        None,
        TokenAmount::zero(),
        None,
        SendFlags::empty(),
        None,
        syscall_exit,
        None,
    );

    test.input = call_params.into();
    test.run_test_expecting(&mut rt, expect, util::PrecompileExit::Success);
}

#[cfg(test)]
mod call_actor_invalid {
    use super::*;

    fn bad_params_inner(mut call_params: CallActorParams, addr: Address, set_addr: bool) {
        let contract = {
            let (init, body) = util::PrecompileTest::test_runner_assembly();
            asm::new_contract("call_actor-precompile-test", &init, &body).unwrap()
        };
        let mut rt = util::construct_and_verify(contract);

        let mut test = util::PrecompileTest {
            precompile_address: util::NativePrecompile::CallActor.eth_address(),
            output_size: 32,
            gas_avaliable: 10_000_000_000u64,
            call_op: util::PrecompileCallOpcode::DelegateCall,
            // overwritten in tests
            expected_return: vec![],
            expected_exit_code: util::PrecompileExit::Success,
            input: call_params.clone().into(),
        };

        if set_addr {
            call_params.set_addr(CallActorParams::EMPTY_PARAM_ADDR_OFFSET, addr.to_bytes());
        }

        test.input = call_params.into();
        test.run_test_expecting(&mut rt, vec![], util::PrecompileExit::Reverted);
    }

    #[test]
    fn no_address() {
        let addr = Address::new_delegated(1234, b"foobarboxy").unwrap();

        let mut call_params = CallActorParams::default();
        call_params.set_addr(CallActorParams::EMPTY_PARAM_ADDR_OFFSET, vec![]);
        bad_params_inner(call_params, addr, false)
    }

    #[test]
    fn invalid_codec() {
        let addr = Address::new_delegated(1234, b"foobarboxy").unwrap();

        let mut call_params = CallActorParams::default();
        call_params.codec(U256([0xff, 0, 0, 0]));
        bad_params_inner(call_params, addr, true)
    }

    #[test]
    fn invalid_method() {
        let addr = Address::new_delegated(1234, b"foobarboxy").unwrap();

        let mut call_params = CallActorParams::default();
        call_params.method(U256([0xff, 0, 0, 0]));
        bad_params_inner(call_params, addr, true)
    }

    #[test]
    fn invalid_params_zero_codec() {
        let addr = Address::new_delegated(1234, b"foobarboxy").unwrap();

        let mut call_params = CallActorParams::default();
        let send_params = vec![0xff; 32];
        call_params
            .set_params(CallActorParams::FIRST_DYNAMIC_OFFSET, Some(send_params))
            .set_addr(CallActorParams::EMPTY_PARAM_ADDR_OFFSET + 32, addr.to_bytes());

        bad_params_inner(call_params, addr, false)
    }

    #[test]
    fn invalid_params_zero_codec_2() {
        let addr = Address::new_delegated(1234, b"foobarboxy").unwrap();

        let mut call_params = CallActorParams::default();
        let send_params = vec![0];
        call_params
            .set_params(CallActorParams::FIRST_DYNAMIC_OFFSET, Some(send_params))
            .set_addr(CallActorParams::EMPTY_PARAM_ADDR_OFFSET + 1, addr.to_bytes());

        bad_params_inner(call_params, addr, false)
    }
}

#[derive(Debug, Clone)]
struct CallActorParams {
    method: U256,
    value: U256,
    flags: U256,
    codec: U256,
    param_offset: U256,
    addr_offset: U256,
    param_len: U256,
    params: Option<Vec<u8>>,
    addr_len: U256,
    addr: Vec<u8>,
}

impl Default for CallActorParams {
    fn default() -> Self {
        Self {
            method: U256::from(0),
            value: U256::from(0),
            flags: U256::from(0),
            codec: U256::from(0),
            // right after static params
            param_offset: U256::from(Self::FIRST_DYNAMIC_OFFSET),
            addr_offset: U256::from(Self::EMPTY_PARAM_ADDR_OFFSET),
            // no len dynamic values
            param_len: U256::from(0),
            params: None,
            addr_len: U256::from(0),
            addr: vec![],
        }
    }
}

impl CallActorParams {
    /// method, value, flags, codec, param_off, addr_off
    /// usually the param offset
    const FIRST_DYNAMIC_OFFSET: usize = 6 * 32;

    const EMPTY_PARAM_ADDR_OFFSET: usize = Self::FIRST_DYNAMIC_OFFSET + 32;

    pub fn set_addr(&mut self, offset: usize, addr: Vec<u8>) -> &mut Self {
        self.addr_len = U256::from(addr.len());
        self.addr_offset = U256::from(offset);
        self.addr = addr;
        self
    }

    pub fn codec(&mut self, codec: U256) -> &mut Self {
        self.codec = codec;
        self
    }

    #[allow(unused)]
    pub fn value(&mut self, value: U256) -> &mut Self {
        self.value = value;
        self
    }

    pub fn method(&mut self, codec: U256) -> &mut Self {
        self.codec = codec;
        self
    }

    pub fn set_params(&mut self, offset: usize, params: Option<Vec<u8>>) -> &mut Self {
        self.param_len = U256::from(params.clone().unwrap_or_default().len());
        self.param_offset = U256::from(offset);
        self.params = params;
        self
    }
}

impl From<CallActorParams> for Vec<u8> {
    // mriise: apologies for whoever needs to change this in the future.
    fn from(src: CallActorParams) -> Self {
        let param_offset = src.param_offset.as_usize();
        let addr_offset = src.addr_offset.as_usize();

        let param_len_usize = src.param_len.as_usize();
        let addr_len_usize = src.addr_len.as_usize();

        let mut out =
            [src.method, src.value, src.flags, src.codec, src.param_offset, src.addr_offset]
                .iter()
                .map(|p| p.to_bytes().to_vec())
                .collect::<Vec<Vec<u8>>>()
                .concat();

        assert_eq!(out.len(), CallActorParams::FIRST_DYNAMIC_OFFSET);
        assert!(param_offset >= out.len());

        let addr_len_offset = addr_offset;
        let addr_begin = addr_len_offset + 32;
        let addr_end = addr_begin + addr_len_usize;

        let param_len_offset = param_offset;
        let param_begin = param_len_offset + 32;
        let param_end = param_begin + param_len_usize;

        out.resize_with(addr_end, || 0);

        let param_len = src.param_len.to_bytes();
        let addr_len = src.addr_len.to_bytes();

        // write single word len values first
        out[param_len_offset..param_len_offset + 32].copy_from_slice(&param_len);
        out[addr_len_offset..addr_len_offset + 32].copy_from_slice(&addr_len);

        // then write actual data immediately after len
        if let Some(params) = src.params.clone() {
            assert!(addr_offset >= param_offset + param_len_usize);
            out[param_begin..param_end].copy_from_slice(&params)
        }
        out[addr_begin..addr_end].copy_from_slice(&src.addr);

        // log::debug!("params\n[{}:32] {}\n[{}:{}] {}", param_len_offset, hex::encode(param_len), param_begin, param_end, hex::encode(&src.params.unwrap_or_default()));
        // log::debug!("address\n[{}:32] {}\n[{}:{}] {}", addr_len_offset, hex::encode(addr_len), addr_begin, addr_end, hex::encode(&src.addr));

        out
    }
}

impl Default for CallActorReturn {
    fn default() -> Self {
        Self { send_exit_code: U256::from(ExitCode::OK.value()), codec: 0, data_offset: 3 * 32, data_size: 0, data: vec![] }
    }
}

#[derive(Debug, PartialEq, Eq)]
struct CallActorReturn {
    send_exit_code: U256,
    codec: u64,
    data_offset: u32,
    data_size: u32,
    data: Vec<u8>,
}

impl From<CallActorReturn> for Vec<u8> {
    fn from(src: CallActorReturn) -> Self {

        // precompile will return negative number for system/syscall errors
        let exit_code = src.send_exit_code;
         
        let codec = U256::from(src.codec);
        let offset = U256::from(src.data_offset);
        let len = U256::from(src.data_size);

        let mut out = [exit_code, codec, offset, len]
            .iter()
            .map(|p| p.to_bytes().to_vec())
            .collect::<Vec<Vec<u8>>>()
            .concat();

        if src.data.len() != src.data_size as usize {
            log::warn!(
                "actor return data length {} does not match specified size {}",
                src.data.len(),
                src.data_size
            )
        }
        out.extend_from_slice(&src.data);
        out
    }
}

fn make_raw_params(bytes: Vec<u8>) -> Option<IpldBlock> {
    if bytes.is_empty() {
        return None;
    }
    Some(IpldBlock { codec: IPLD_RAW, data: bytes })
}

#[test]
fn call_actor_solidity() {
    // solidity
    let contract_hex = include_str!("contracts/CallActorPrecompile.hex");
    // let mut contract_rt = new_call_actor_contract();
    let contract_address = EthAddress(util::CONTRACT_ADDRESS);
    let mut tester = ContractTester::new(contract_address, 111, contract_hex);

    // call_actor_id
    {
        let params =
            CONTRACT.call_actor_id(0, ethers::types::U256::zero(), 0, 0, Bytes::default(), 101);

        let expected_return = vec![0xff, 0xfe];
        tester.rt.expect_send_generalized(
            Address::new_id(101),
            0,
            None,
            TokenAmount::from_atto(0),
            Some(9843750),
            SendFlags::empty(),
            Some(IpldBlock { codec: 0, data: expected_return.clone() }),
            ExitCode::OK,
            None,
        );

        let (success, exit, codec, ret_val): (bool, ethers::types::I256, u64, Bytes) =
            tester.call(params);

        assert!(success);
        assert_eq!(exit, I256::from(0));
        assert_eq!(codec, 0);
        assert_eq!(&ret_val, &expected_return, "got {}", hex::encode(&ret_val));
    }
    tester.rt.reset();
    // call_actor
    {
        log::warn!("new test");
        // EVM actor
        let evm_target = FILAddress::new_id(10101);
        let evm_del = EthAddress(util::CONTRACT_ADDRESS).try_into().unwrap();
        tester.rt.add_delegated_address(evm_target, evm_del);

        let to_address = {
            let subaddr = hex_literal::hex!("b0ba000000000000000000000000000000000000");
            Address::new_delegated(EAM_ACTOR_ID, &subaddr).unwrap()
        };
        let params = CONTRACT.call_actor_address(
            0,
            ethers::types::U256::zero(),
            0,
            0,
            Bytes::default(),
            to_address.to_bytes().into(),
        );

        let expected_return = vec![0xff, 0xfe];
        tester.rt.expect_send_generalized(
            to_address,
            0,
            None,
            TokenAmount::from_atto(0),
            Some(9843750),
            SendFlags::empty(),
            Some(IpldBlock { codec: 0, data: expected_return.clone() }),
            ExitCode::OK,
            None,
        );

        let (success, exit, codec, ret_val): (bool, ethers::types::I256, u64, Bytes) =
            tester.call(params);

        assert!(success);
        assert_eq!(exit, I256::from(0));
        assert_eq!(codec, 0);
        assert_eq!(&ret_val, &expected_return, "got {}", hex::encode(&ret_val));
    }
}

#[test]
fn call_actor_send_solidity() {
    // solidity
    let contract_hex = include_str!("contracts/CallActorPrecompile.hex");
    // let mut contract_rt = new_call_actor_contract();
    let contract_address = EthAddress(util::CONTRACT_ADDRESS);
    let mut tester = ContractTester::new(contract_address, 111, contract_hex);

    // send 1 atto Fil (this should be a full integration tests rly)
    {
        let params =
            CONTRACT.call_actor_id(0, ethers::types::U256::from(1), 0, 0, Bytes::default(), 101);

        tester.rt.add_id_address(
            Address::new_delegated(12345, b"foobarboxy").unwrap(),
            Address::new_id(101),
        );

        tester.rt.add_balance(TokenAmount::from_atto(100));

        let expected_return = vec![0xff, 0xfe];
        tester.rt.expect_send_generalized(
            Address::new_id(101),
            0,
            None,
            TokenAmount::from_atto(1),
            Some(9843750),
            SendFlags::empty(),
            Some(IpldBlock { codec: 0, data: expected_return.clone() }),
            ExitCode::OK,
            None,
        );

        let (success, exit, codec, ret_val): (bool, ethers::types::I256, u64, Bytes) =
            tester.call(params);

        assert!(success);
        assert_eq!(exit, I256::from(0));
        assert_eq!(codec, 0);
        assert_eq!(&ret_val, &expected_return, "got {}", hex::encode(&ret_val));
        assert_eq!(tester.rt.get_balance(), TokenAmount::from_atto(99));
    }
}

pub(crate) struct ContractTester {
    rt: MockRuntime,
    _address: EthAddress,
}

impl ContractTester {
    fn new(addr: EthAddress, id: u64, contract_hex: &str) -> Self {
        init_logging().ok();

        let mut rt = MockRuntime::default();
        let params = evm::ConstructorParams {
            creator: EthAddress::from_id(EAM_ACTOR_ID),
            initcode: hex::decode(contract_hex).unwrap().into(),
        };
        rt.add_id_address(addr.try_into().unwrap(), FILAddress::new_id(id));

        // invoke constructor
        rt.expect_validate_caller_addr(vec![INIT_ACTOR_ADDR]);
        rt.set_caller(*INIT_ACTOR_CODE_ID, INIT_ACTOR_ADDR);

        rt.set_origin(FILAddress::new_id(0));
        // first actor created is 0
        rt.add_delegated_address(
            Address::new_id(0),
            Address::new_delegated(EAM_ACTOR_ID, &addr.0).unwrap(),
        );

        assert!(rt
            .call::<evm::EvmContractActor>(
                evm::Method::Constructor as u64,
                IpldBlock::serialize_cbor(&params).unwrap(),
            )
            .unwrap()
            .is_none());

        rt.verify();
        rt.reset();
        Self { rt, _address: addr }
    }

    fn call<Returns: Detokenize>(&mut self, call: TestContractCall<Returns>) -> Returns {
        let input = call.calldata().expect("Should have calldata.").to_vec();
        let input =
            IpldBlock::serialize_cbor(&BytesSer(&input)).expect("failed to serialize input data");

        self.rt.expect_validate_caller_any();
        self.rt.expect_gas_available(10_000_000);
        self.rt.expect_gas_available(10_000_000);

        let BytesDe(result) = self
            .rt
            .call::<evm::EvmContractActor>(evm::Method::InvokeContract as u64, input)
            .unwrap()
            .unwrap()
            .deserialize()
            .unwrap();

        decode_function_data(&call.function, result, false).unwrap()
    }
}
