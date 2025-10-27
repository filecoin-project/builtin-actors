use fil_actor_evm as evm;
use fil_actors_evm_shared::address::EthAddress;
use fil_actors_runtime::test_utils::MockRuntime;
use fvm_ipld_encoding::ipld_block::IpldBlock;
use fvm_shared::error::ExitCode;

mod util;

#[test]
fn tuple_cap_allows_64() {
    let rt = util::construct_and_verify(vec![]);
    const GAS_BASE_APPLY7702: i64 = 0;
    const GAS_PER_AUTH_TUPLE: i64 = 10_000;
    // Expect base + per-tuple charged once; runtime mock lumps multiple identical charges; we only assert the first two calls.
    rt.expect_gas_charge(GAS_BASE_APPLY7702);
    rt.expect_gas_charge(GAS_PER_AUTH_TUPLE);

    // Build 64 tuples with distinct nonces so recovered authorities are unique.
    let dst = EthAddress(hex_literal::hex!("bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"));
    let mut list = Vec::with_capacity(64);
    for n in 0..64u64 {
        list.push(evm::DelegationParam { chain_id: 0, address: dst, nonce: n, y_parity: 0, r: vec![1u8;32], s: vec![1u8;32] });
    }
    let params = evm::ApplyAndCallParams { list, call: evm::ApplyCall { to: dst, value: vec![], input: vec![] } };
    rt.expect_validate_caller_any();
    let res = rt.call::<evm::EvmContractActor>(evm::Method::ApplyAndCall as u64, IpldBlock::serialize_dag_cbor(&params).unwrap());
    assert!(res.is_ok());
}

#[test]
fn tuple_cap_rejects_65() {
    let rt = util::construct_and_verify(vec![]);
    const GAS_BASE_APPLY7702: i64 = 0;
    const GAS_PER_AUTH_TUPLE: i64 = 10_000;
    rt.expect_gas_charge(GAS_BASE_APPLY7702);
    rt.expect_gas_charge(GAS_PER_AUTH_TUPLE);

    let dst = EthAddress(hex_literal::hex!("cccccccccccccccccccccccccccccccccccccccc"));
    let mut list = Vec::with_capacity(65);
    for n in 0..65u64 {
        list.push(evm::DelegationParam { chain_id: 0, address: dst, nonce: n, y_parity: 0, r: vec![1u8;32], s: vec![1u8;32] });
    }
    let params = evm::ApplyAndCallParams { list, call: evm::ApplyCall { to: dst, value: vec![], input: vec![] } };
    rt.expect_validate_caller_any();
    let res = rt.call::<evm::EvmContractActor>(evm::Method::ApplyAndCall as u64, IpldBlock::serialize_dag_cbor(&params).unwrap());
    assert!(res.is_err());
    assert_eq!(res.err().unwrap().exit_code(), ExitCode::USR_ILLEGAL_ARGUMENT);
}

