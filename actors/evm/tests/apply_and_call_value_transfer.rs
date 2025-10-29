use cid::Cid;
use fil_actor_evm as evm;
use fil_actors_evm_shared::address::EthAddress;
use fvm_ipld_blockstore::Blockstore;
use fvm_ipld_encoding::ipld_block::IpldBlock;
use fvm_shared::{
    address::Address as FilAddress, econ::TokenAmount, error::ExitCode, sys::SendFlags,
};

mod util;

// ApplyAndCall should fail (status=0) when the pre-call value transfer to the delegated EOA fails.
// This simulates insufficient funds or other transfer errors. The actor must not proceed to
// InvokeAsEoa in this case.
#[test]
fn apply_and_call_value_transfer_failure_short_circuits() {
    let mut rt = util::construct_and_verify(vec![]);

    // Deterministic recovered authority A.
    let mut pk_a = [0u8; 65];
    pk_a[0] = 0x04;
    for b in pk_a.iter_mut().skip(1) {
        *b = 0xA7;
    }
    // Derive A's EthAddress as the actor would: keccak(pubkey[1:])[12:]
    use fil_actors_runtime::test_utils::hash as rt_hash;
    use fvm_shared::crypto::hash::SupportedHashes;
    let (keccak_a, _) = rt_hash(SupportedHashes::Keccak256, &pk_a[1..]);
    let mut a20 = [0u8; 20];
    a20.copy_from_slice(&keccak_a[12..32]);
    let a_eth = EthAddress(a20);

    // Delegate B is the receiver EVM actor (ID 0) with known f4 address set by util::construct_and_verify.
    let b_eth: EthAddress = EthAddress(util::CONTRACT_ADDRESS);

    // Force signature recovery to return A's pubkey for the authorization tuple.
    rt.recover_secp_pubkey_fn = Box::new(move |_, _| Ok(pk_a));

    // Intrinsic gas placeholder expectations.
    // No gas expectations in tests (behavioral only).

    // Build ApplyAndCall with a single tuple (A -> B) and call A (EOA) with non-zero value.
    let list = vec![evm::DelegationParam {
        chain_id: 0,
        address: b_eth,
        nonce: 0,
        y_parity: 0,
        r: vec![1u8; 32],
        s: vec![1u8; 32],
    }];
    let call_value = TokenAmount::from_atto(999u64);
    let params = evm::ApplyAndCallParams {
        list,
        // Encode the value as big-endian magnitude bytes (no sign), e.g., 999 -> 0x03E7
        call: evm::ApplyCall { to: a_eth, value: vec![0x03, 0xE7], input: vec![] },
    };

    // Expect GetBytecode(delegate=B) to return some code CID (content irrelevant for this test).
    let bytecode_cid = Cid::try_from("baeaikaia").unwrap();
    rt.store.put_keyed(&bytecode_cid, &[]).unwrap();
    rt.expect_send(
        FilAddress::new_id(0),
        evm::Method::GetBytecode as u64,
        None,
        TokenAmount::from_whole(0),
        None,
        SendFlags::READ_ONLY,
        IpldBlock::serialize_cbor(&bytecode_cid).unwrap(),
        ExitCode::OK,
        None,
    );

    // Expect value transfer to authority (EOA) to fail.
    let authority_f4: FilAddress = a_eth.into();
    rt.expect_send_simple(
        authority_f4,
        fvm_shared::METHOD_SEND,
        None,
        call_value.clone(),
        None,
        ExitCode::USR_ILLEGAL_STATE,
    );

    // Note: No expectation for InvokeAsEoa should be needed; we should short-circuit before that.

    // Call ApplyAndCall; should return OK envelope with embedded status=0 and no output data.
    rt.expect_validate_caller_any();
    let res = rt.call::<evm::EvmContractActor>(
        evm::Method::ApplyAndCall as u64,
        IpldBlock::serialize_dag_cbor(&params).unwrap(),
    );
    assert!(res.is_ok(), "ApplyAndCall envelope must succeed");

    // Verify the expected sends were executed and no unexpected InvokeAsEoa occurred.
    rt.verify();
}
