use fil_actor_evm as evm;
use fil_actors_evm_shared::address::EthAddress;
use fil_actors_runtime::test_utils::MockRuntime;
use fvm_ipld_encoding::ipld_block::IpldBlock;
use fvm_shared::{address::Address as FilAddress, econ::TokenAmount, error::ExitCode, sys::SendFlags};

mod util;

#[test]
fn mapping_and_nonce_do_not_persist_on_outer_call_failure() {
    let rt = util::construct_and_verify(vec![]);

    // Intrinsic gas charges
    const GAS_BASE_APPLY7702: i64 = 0;
    const GAS_PER_AUTH_TUPLE: i64 = 10_000;
    rt.expect_gas_charge(GAS_BASE_APPLY7702);
    rt.expect_gas_charge(GAS_PER_AUTH_TUPLE);

    // First attempt: tuple for authority; outer call fails (EVM InvokeContract returns error).
    let dst = EthAddress(hex_literal::hex!("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"));
    let list = vec![evm::DelegationParam { chain_id: 0, address: dst, nonce: 0, y_parity: 0, r: vec![1u8;32], s: vec![1u8;32] }];
    let params = evm::ApplyAndCallParams { list: list.clone(), call: evm::ApplyCall { to: dst, value: vec![], input: vec![0x01] } };
    // Set destination as EVM actor and simulate InvokeContract failure.
    use fil_actors_runtime::test_utils::EVM_ACTOR_CODE_ID;
    let dst_f4: FilAddress = dst.into();
    let dst_id = FilAddress::new_id(0x444u64);
    rt.set_delegated_address(dst_id.id().unwrap(), dst_f4);
    rt.actor_code_cids.borrow_mut().insert(dst_id, *EVM_ACTOR_CODE_ID);
    rt.expect_send(
        dst_id,
        evm::Method::InvokeContract as u64,
        Some(IpldBlock { codec: fvm_ipld_encoding::IPLD_RAW, data: vec![0x01] }),
        TokenAmount::from_whole(0),
        None,
        SendFlags::empty(),
        None,
        ExitCode::USR_ILLEGAL_STATE,
        None,
    );
    rt.expect_validate_caller_any();
    let res = rt.call::<evm::EvmContractActor>(
        evm::Method::ApplyAndCall as u64,
        IpldBlock::serialize_dag_cbor(&params).unwrap(),
    );
    assert!(res.is_err());

    // Second attempt: reuse the same tuple (nonce=0). If the failed attempt had persisted,
    // this would now be a nonce mismatch. Expect it to proceed to send.
    const GAS_BASE_APPLY7702_2: i64 = 0;
    const GAS_PER_AUTH_TUPLE_2: i64 = 10_000;
    rt.expect_gas_charge(GAS_BASE_APPLY7702_2);
    rt.expect_gas_charge(GAS_PER_AUTH_TUPLE_2);
    // Simulate a successful InvokeContract this time (empty return).
    rt.expect_send(
        dst_id,
        evm::Method::InvokeContract as u64,
        Some(IpldBlock { codec: fvm_ipld_encoding::IPLD_RAW, data: vec![0x01] }),
        TokenAmount::from_whole(0),
        None,
        SendFlags::empty(),
        Some(IpldBlock { codec: fvm_ipld_encoding::IPLD_RAW, data: vec![] }),
        ExitCode::OK,
        None,
    );
    rt.expect_validate_caller_any();
    let res2 = rt.call::<evm::EvmContractActor>(
        evm::Method::ApplyAndCall as u64,
        IpldBlock::serialize_dag_cbor(&params).unwrap(),
    );
    assert!(res2.is_ok());
}

