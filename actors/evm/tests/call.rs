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
use evm::EVM_CONTRACT_REVERTED;
use fil_actor_evm as evm;
use fil_actors_runtime::{test_utils::*, EAM_ACTOR_ID, INIT_ACTOR_ADDR};
use fvm_ipld_encoding::ipld_block::IpldBlock;
use fvm_ipld_encoding::{BytesDe, BytesSer, IPLD_RAW};
use fvm_shared::address::Address as FILAddress;
use fvm_shared::address::Address;
use fvm_shared::bigint::Zero;
use fvm_shared::econ::TokenAmount;
use fvm_shared::error::ExitCode;
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
    assert_eq!(result, Some(IpldBlock { codec: 0x71, data: "foobar".into() }));

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
        32 * CALLACTOR_NUM_PARAMS + target_bytes.len() + proxy_call_input_data.len(),
        contract_params.len(),
        "unexpected input length"
    );

    // expected return data
    // Test with a codec _other_ than DAG_CBOR, to make sure we are actually passing the returned codec
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
        );
    }

    // output bytes are padded to nearest 32 byte
    let mut v = vec![0; 32];
    v[..4].copy_from_slice(&send_return.data);

    let expect = CallActorReturn {
        exit_code,
        codec: send_return.codec,
        data_offset: 96,
        data_size: send_return.data.len() as u32,
        data: v,
    };

    let (expected_exit, expected_out) = if valid_call_input {
        (util::PrecompileExit::Success, expect.into_vec())
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

#[derive(Debug, PartialEq, Eq)]
struct CallActorReturn {
    exit_code: ExitCode,
    codec: u64,
    data_offset: u32,
    data_size: u32,
    data: Vec<u8>,
}

impl CallActorReturn {
    fn into_vec(self) -> Vec<u8> {
        assert_eq!(self.data.len() % 32, 0);

        let exit = U256::from(self.exit_code.value());
        let codec = U256::from(self.codec);
        let data_offset = U256::from(self.data_offset);
        let data_size = U256::from(self.data_size);

        let mut out = [exit, codec, data_offset, data_size]
            .iter()
            .map(|p| p.to_bytes().to_vec())
            .collect::<Vec<Vec<u8>>>()
            .concat();
        out.extend_from_slice(&self.data);
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
        );

        let (success, exit, codec, ret_val): (bool, ethers::types::I256, u64, Bytes) =
            tester.call(params);

        assert!(success);
        assert_eq!(exit, I256::from(0));
        assert_eq!(codec, 0);
        assert_eq!(&ret_val, &expected_return, "got {}", hex::encode(&ret_val));
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
