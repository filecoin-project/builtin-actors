use cid::Cid;
use fil_actor_evm as evm;
use fil_actors_evm_shared::address::EthAddress;
use fil_actors_runtime::test_utils::MockRuntime;
use fvm_ipld_encoding::ipld_block::IpldBlock;
use fvm_shared::econ::TokenAmount;
use fvm_shared::error::ExitCode;
use fvm_shared::sys::SendFlags;

mod util;

#[test]
fn apply_and_call_rejects_invalid_authorizations() {
    let rt = util::construct_and_verify(vec![]);

    // Build ApplyAndCall with a single tuple (invalid y_parity; validator should reject).
    let authority = EthAddress(hex_literal::hex!("00112233445566778899aabbccddeeff00112233"));
    let list = vec![evm::DelegationParam {
        chain_id: 0,
        address: authority,
        nonce: 0,
        y_parity: 2,
        r: vec![0u8; 32],
        s: vec![0u8; 32],
    }];
    let params = evm::ApplyAndCallParams {
        list: list.clone(),
        call: evm::ApplyCall { to: authority, value: vec![0u8], input: vec![] },
    };

    // Call ApplyAndCall; expect error with same exit code propagated.
    rt.expect_validate_caller_any();
    let res = rt.call::<evm::EvmContractActor>(
        evm::Method::ApplyAndCall as u64,
        IpldBlock::serialize_dag_cbor(&params).unwrap(),
    );
    assert!(res.is_err());
    let err = res.err().unwrap();
    assert_eq!(err.exit_code(), ExitCode::USR_ILLEGAL_ARGUMENT);
}

#[test]
fn apply_and_call_propagates_outer_call_failure() {
    let rt = util::construct_and_verify(vec![]);

    // Build ApplyAndCall with valid-looking tuple and EVM destination.
    let dst = EthAddress(hex_literal::hex!("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"));
    let list = vec![evm::DelegationParam {
        chain_id: 0,
        address: dst,
        nonce: 0,
        y_parity: 0,
        r: vec![1u8; 32],
        s: vec![1u8; 32],
    }];
    let params = evm::ApplyAndCallParams {
        list: list.clone(),
        call: evm::ApplyCall { to: dst, value: vec![0u8], input: vec![0x01] },
    };

    // Destination is EVM contract: expect InvokeContract to fail.
    use fil_actors_runtime::test_utils::EVM_ACTOR_CODE_ID;
    use fvm_shared::address::Address as FilAddress;
    let dst_f4: FilAddress = dst.into();
    let dst_id = FilAddress::new_id(0x333u64);
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
        ExitCode::USR_ILLEGAL_STATE, // simulate a failure
        None,
    );

    // Call ApplyAndCall; expect error propagated.
    rt.expect_validate_caller_any();
    let res = rt.call::<evm::EvmContractActor>(
        evm::Method::ApplyAndCall as u64,
        IpldBlock::serialize_dag_cbor(&params).unwrap(),
    );
    assert!(res.is_err());
}
