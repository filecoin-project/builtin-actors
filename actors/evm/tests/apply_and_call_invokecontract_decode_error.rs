use fil_actor_evm as evm;
use fil_actors_evm_shared::address::EthAddress;
use fvm_ipld_encoding::ipld_block::IpldBlock;
use fvm_shared::{
    IPLD_RAW, address::Address as FilAddress, econ::TokenAmount, error::ExitCode, sys::SendFlags,
};

mod util;

// ApplyAndCall: direct EVM call branch (InvokeContract) returns ExitCode::OK
// but the return payload cannot be decoded into InvokeContractReturn.
// Expected behavior: ApplyAndCall must return OK with status=0 and not abort
// after applying and flushing delegation state.
#[test]
fn apply_and_call_invokecontract_decode_error_maps_to_status_zero() {
    let mut rt = util::construct_and_verify(vec![]);

    // Prepare a valid authorization (A->B) even though we'll call the EVM actor directly.
    let mut pk_a = [0u8; 65];
    pk_a[0] = 0x04;
    for b in pk_a.iter_mut().skip(1) {
        *b = 0xC1;
    }
    rt.recover_secp_pubkey_fn = Box::new(move |_, _| Ok(pk_a));

    // Delegate B = this EVM actor; outer call targets B directly (is_evm=true).
    let b_eth: EthAddress = EthAddress(util::CONTRACT_ADDRESS);

    let params = evm::ApplyAndCallParams {
        list: vec![evm::DelegationParam {
            chain_id: 0,
            address: b_eth,
            nonce: 0,
            y_parity: 0,
            r: vec![1u8; 32],
            s: vec![1u8; 32],
        }],
        call: evm::ApplyCall { to: b_eth, value: vec![], input: vec![0x01] },
    };

    // Ensure destination resolves to EVM.
    use fil_actors_runtime::test_utils::EVM_ACTOR_CODE_ID;
    let b_f4: FilAddress = b_eth.into();
    let b_id = FilAddress::new_id(0xBEEF_u64);
    rt.set_delegated_address(b_id.id().unwrap(), b_f4);
    rt.actor_code_cids.borrow_mut().insert(b_id, *EVM_ACTOR_CODE_ID);

    // Expect InvokeContract to return success, but with an invalid (non-CBOR) payload
    // so decoding into InvokeContractReturn fails.
    let invalid_raw = vec![0xDE, 0xAD, 0xBE, 0xEF];
    rt.expect_send(
        b_id,
        evm::Method::InvokeContract as u64,
        Some(IpldBlock { codec: IPLD_RAW, data: vec![0x01] }),
        TokenAmount::from_whole(0),
        None,
        SendFlags::empty(),
        Some(IpldBlock { codec: IPLD_RAW, data: invalid_raw }),
        ExitCode::OK,
        None,
    );

    rt.expect_validate_caller_any();
    let ret_blk = rt
        .call::<evm::EvmContractActor>(
            evm::Method::ApplyAndCall as u64,
            IpldBlock::serialize_dag_cbor(&params).unwrap(),
        )
        .unwrap()
        .unwrap();
    let out: evm::ApplyAndCallReturn = ret_blk.deserialize().unwrap();
    assert_eq!(out.status, 0, "expected status=0 on InvokeContract decode error");
}
