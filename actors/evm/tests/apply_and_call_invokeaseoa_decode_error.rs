use cid::Cid;
use fil_actor_evm as evm;
use fil_actors_evm_shared::address::EthAddress;
use fil_actors_runtime::test_utils::SendOutcome;
use fvm_ipld_blockstore::Blockstore;
use fvm_ipld_encoding::ipld_block::IpldBlock;
use fvm_shared::{
    IPLD_RAW, address::Address as FilAddress, econ::TokenAmount, error::ExitCode, sys::SendFlags,
};

mod util;

// ApplyAndCall: InvokeAsEoa returns ExitCode::OK but the return cannot be decoded
// into InvokeContractReturn. ApplyAndCall must not abort after the pre-call flush;
// it must return OK with status=0 and preserve mapping/nonces (nonce mismatch on re-apply).
#[test]
fn apply_and_call_invokeaseoa_decode_error_maps_to_status_zero() {
    let mut rt = util::construct_and_verify(vec![]);

    // Prepare deterministic authority A (by pubkey) and delegate B (this EVM actor).
    let mut pk_a = [0u8; 65];
    pk_a[0] = 0x04;
    for b in pk_a.iter_mut().skip(1) {
        *b = 0xA7;
    }
    rt.recover_secp_pubkey_fn = Box::new(move |_, _| Ok(pk_a));

    // Derive authority A's Eth address from pubkey.
    use fil_actors_runtime::test_utils::hash as rt_hash;
    use fvm_shared::crypto::hash::SupportedHashes;
    let (keccak_a, _) = rt_hash(SupportedHashes::Keccak256, &pk_a[1..]);
    let mut a20 = [0u8; 20];
    a20.copy_from_slice(&keccak_a[12..32]);
    let a_eth = EthAddress(a20);

    let b_eth: EthAddress = EthAddress(util::CONTRACT_ADDRESS);

    let list = vec![evm::DelegationParam {
        chain_id: 0,
        address: b_eth,
        nonce: 0,
        y_parity: 0,
        r: vec![1u8; 32],
        s: vec![1u8; 32],
    }];
    let params = evm::ApplyAndCallParams {
        list,
        call: evm::ApplyCall { to: a_eth, value: vec![], input: vec![] },
    };

    // GetBytecode(delegate=B) succeeds with a code CID.
    let code_cid = Cid::try_from("baeaikaia").unwrap();
    rt.store.put_keyed(&code_cid, &[]).unwrap();
    rt.expect_send(
        FilAddress::new_id(0),
        evm::Method::GetBytecode as u64,
        None,
        TokenAmount::from_whole(0),
        None,
        SendFlags::READ_ONLY,
        IpldBlock::serialize_cbor(&code_cid).unwrap(),
        ExitCode::OK,
        None,
    );

    // InvokeAsEoa returns success but with a non-CBOR (invalid) payload, triggering decode error.
    let invalid_raw = vec![0xDE, 0xAD, 0xBE, 0xEF];
    rt.expect_send_any_params(
        rt.receiver,
        evm::Method::InvokeAsEoa as u64,
        TokenAmount::from_whole(0),
        None,
        SendFlags::default(),
        SendOutcome {
            send_return: Some(IpldBlock { codec: IPLD_RAW, data: invalid_raw }),
            exit_code: ExitCode::OK,
            send_error: None,
        },
    );

    // Call ApplyAndCall and decode the return; expect status=0.
    rt.expect_validate_caller_any();
    let ret_blk = rt
        .call::<evm::EvmContractActor>(
            evm::Method::ApplyAndCall as u64,
            IpldBlock::serialize_dag_cbor(&params).unwrap(),
        )
        .unwrap()
        .unwrap();
    let out: evm::ApplyAndCallReturn = ret_blk.deserialize().unwrap();
    assert_eq!(out.status, 0, "expected status=0 on decode error of InvokeAsEoa return");
    assert!(out.output_data.is_empty());
    rt.verify();

    // Re-apply with nonce=0 must now fail (nonce bumped and persisted).
    let params_again = evm::ApplyAndCallParams {
        list: vec![evm::DelegationParam {
            chain_id: 0,
            address: b_eth,
            nonce: 0,
            y_parity: 0,
            r: vec![2u8; 32],
            s: vec![2u8; 32],
        }],
        call: evm::ApplyCall { to: a_eth, value: vec![], input: vec![] },
    };
    rt.expect_validate_caller_any();
    let res2 = rt.call::<evm::EvmContractActor>(
        evm::Method::ApplyAndCall as u64,
        IpldBlock::serialize_dag_cbor(&params_again).unwrap(),
    );
    assert!(res2.is_err(), "re-applying nonce=0 should fail after first apply");
}

