use fil_actor_evm::{
    interpreter::{address::EthAddress, U256},
    DelegateCallParams, Method,
};
use fil_actors_runtime::{runtime::EMPTY_ARR_CID, test_utils::EVM_ACTOR_CODE_ID};
use fvm_ipld_encoding::{ipld_block::IpldBlock, BytesSer, RawBytes, DAG_CBOR};
use fvm_shared::{
    address::Address as FILAddress, econ::TokenAmount, error::ExitCode, sys::SendFlags,
};
use num_traits::Zero;

mod asm;
mod util;

#[allow(dead_code)]
pub fn delegatecall_proxy_contract() -> Vec<u8> {
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
push1 0x05
# value
push1 0x00
# dest address
push1 0x00
calldataload
# gas
push4 0xffffffff
# do the call
delegatecall

# return result through
returndatasize
push1 0x00
push1 0x00
returndatacopy
returndatasize
push1 0x00
return
"#;

    asm::new_contract("delegatecall-proxy", init, body).unwrap()
}

#[test]
fn test_delegate_call_caller() {
    let contract = delegatecall_proxy_contract();

    // construct the proxy
    let mut rt = util::construct_and_verify(contract);

    // create a mock target and proxy a call through the proxy
    let target_id = 0x100;
    let target = FILAddress::new_id(target_id);
    let evm_target = EthAddress(hex_literal::hex!("deadbeefdeadbeefdeadbeefdeadbeefdeadbeef"));
    let f4_target: FILAddress = evm_target.try_into().unwrap();
    rt.actor_code_cids.insert(target, *EVM_ACTOR_CODE_ID);
    rt.add_delegated_address(target, f4_target);
    rt.receiver = target;

    // set caller that is expected to persist through to subcall
    let caller = FILAddress::new_id(0x111);
    let evm_caller = EthAddress(util::CONTRACT_ADDRESS);
    let f4_caller = evm_caller.try_into().unwrap();
    rt.add_delegated_address(caller, f4_caller);
    rt.caller = caller;

    let evm_target_word = evm_target.as_evm_word();

    // dest + method 0 + single byte of data
    let mut contract_params = vec![0u8; 37];
    evm_target_word.to_big_endian(&mut contract_params[..32]);
    contract_params[36] = 0x01;

    // dest 0 in this test has code cid EMPTY_ARR_CID

    let proxy_call_contract_params = DelegateCallParams {
        code: EMPTY_ARR_CID,
        input: vec![0, 0, 0, 0, 0x01],
        caller: evm_caller,
        value: TokenAmount::from_whole(123),
    };
    let proxy_call_input_data = Some(IpldBlock {
        codec: DAG_CBOR,
        data: RawBytes::serialize(proxy_call_contract_params)
            .expect("failed to serialize delegate call params")
            .to_vec(),
    });

    // expected return data
    let return_data = U256::from(0x42);

    rt.set_value(TokenAmount::from_whole(123));
    rt.expect_gas_available(10_000_000_000u64);
    rt.expect_send_generalized(
        target,
        Method::GetBytecode as u64,
        None,
        TokenAmount::zero(),
        None,
        SendFlags::READ_ONLY,
        IpldBlock::serialize_cbor(&EMPTY_ARR_CID).expect("failed to serialize bytecode hash"),
        ExitCode::OK,
    );

    rt.expect_send_generalized(
        target,
        Method::InvokeContractDelegate as u64,
        proxy_call_input_data,
        TokenAmount::zero(),
        Some(0xffffffff),
        SendFlags::empty(),
        IpldBlock::serialize_cbor(&BytesSer(&return_data.to_bytes()))
            .expect("failed to serialize return data"),
        ExitCode::OK,
    );

    let result = util::invoke_contract(&mut rt, &contract_params);
    assert_eq!(U256::from_big_endian(&result), return_data);
}
