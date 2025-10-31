use cid::Cid;
use fil_actor_evm as evm;
use fil_actors_evm_shared::address::EthAddress;
use fvm_ipld_encoding::ipld_block::IpldBlock;
use fvm_shared::{
    address::Address as FilAddress, econ::TokenAmount, error::ExitCode, sys::SendFlags,
};

mod util;

// ApplyAndCall: delegate has no code (GetBytecode returns OK with None).
// Expect ApplyAndCall to return OK with status=0 (delegated execution could not proceed).
#[test]
fn apply_and_call_delegate_no_code_maps_to_status_zero() {
    let mut rt = util::construct_and_verify(vec![]);

    // Deterministic authority A and delegate B (this EVM actor).
    let mut pk_a = [0u8; 65];
    pk_a[0] = 0x04;
    for b in pk_a.iter_mut().skip(1) {
        *b = 0xAC;
    }
    rt.recover_secp_pubkey_fn = Box::new(move |_, _| Ok(pk_a));

    use fil_actors_runtime::test_utils::hash as rt_hash;
    use fvm_shared::crypto::hash::SupportedHashes;
    let (keccak_a, _) = rt_hash(SupportedHashes::Keccak256, &pk_a[1..]);
    let mut a20 = [0u8; 20];
    a20.copy_from_slice(&keccak_a[12..32]);
    let a_eth = EthAddress(a20);

    let b_eth: EthAddress = EthAddress(util::CONTRACT_ADDRESS);

    // Apply A->B and call A (EOA) so we take the delegated path.
    let params = evm::ApplyAndCallParams {
        list: vec![evm::DelegationParam {
            chain_id: 0,
            address: b_eth,
            nonce: 0,
            y_parity: 0,
            r: vec![1u8; 32],
            s: vec![1u8; 32],
        }],
        call: evm::ApplyCall { to: a_eth, value: vec![], input: vec![] },
    };

    // Stub GetBytecode(delegate=B) to return success but with None (no code CID).
    // We encode an Option<Cid>::None to exercise the "no code" branch cleanly.
    let none_cid: Option<Cid> = None;
    rt.expect_send(
        FilAddress::new_id(0),
        evm::Method::GetBytecode as u64,
        None,
        TokenAmount::from_whole(0),
        None,
        SendFlags::READ_ONLY,
        IpldBlock::serialize_cbor(&none_cid).unwrap(),
        ExitCode::OK,
        None,
    );

    // No InvokeAsEoa expected (we shouldn't try to execute).

    rt.expect_validate_caller_any();
    let ret_blk = rt
        .call::<evm::EvmContractActor>(
            evm::Method::ApplyAndCall as u64,
            IpldBlock::serialize_dag_cbor(&params).unwrap(),
        )
        .unwrap()
        .unwrap();
    let out: evm::ApplyAndCallReturn = ret_blk.deserialize().unwrap();
    assert_eq!(out.status, 0, "expected status=0 when delegate has no code");
    assert!(out.output_data.is_empty());
    rt.verify();
}
