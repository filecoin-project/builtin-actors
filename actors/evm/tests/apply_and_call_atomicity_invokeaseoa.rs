use cid::Cid;
use fil_actor_evm as evm;
use fil_actors_evm_shared::address::EthAddress;
use fil_actors_runtime::test_utils::SendOutcome;
use fvm_ipld_blockstore::Blockstore;
use fvm_ipld_encoding::ipld_block::IpldBlock;
use fvm_shared::{IPLD_RAW, address::Address as FilAddress, econ::TokenAmount, error::ExitCode, sys::SendFlags};

mod util;

// CRITICAL: Atomicity on InvokeAsEoa revert
//
// This test reproduces the atomicity bug identified in the review:
// - ApplyAndCall must persist delegation mapping and nonce bumps even if the outer call reverts.
// - When the outer call targets an EOA with an active delegation, ApplyAndCall calls InvokeAsEoa.
// - If InvokeAsEoa returns a failure exit code, ApplyAndCall must NOT abort; it must return OK
//   with embedded status=0 and keep the state changes.
//
// Current implementation uses `?` on the internal send to InvokeAsEoa, causing the entire
// ApplyAndCall to abort and roll back state. This test asserts the expected behavior and will
// fail against the buggy implementation, validating the review.
#[test]
fn apply_and_call_invokeaseoa_revert_keeps_state_and_returns_ok() {
    // Construct EVM actor with empty bytecode.
    let mut rt = util::construct_and_verify(vec![]);

    // Prepare a deterministic recovered authority A.
    let mut pk_a = [0u8; 65];
    pk_a[0] = 0x04;
    for b in pk_a.iter_mut().skip(1) { *b = 0xA1; }
    // Derive A's EthAddress as the actor would: keccak(pubkey[1:])[12:]
    use fil_actors_runtime::test_utils::hash as rt_hash;
    use fvm_shared::crypto::hash::SupportedHashes;
    let (keccak_a, _) = rt_hash(SupportedHashes::Keccak256, &pk_a[1..]);
    let mut a20 = [0u8; 20];
    a20.copy_from_slice(&keccak_a[12..32]);
    let a_eth = EthAddress(a20);

    // Delegate B is the receiver EVM actor (ID 0) with known ETH f4 address set by util::construct_and_verify.
    let b_eth: EthAddress = EthAddress(util::CONTRACT_ADDRESS);

    // Force signature recovery to return A's pubkey for the authorization tuple.
    rt.recover_secp_pubkey_fn = Box::new(move |_, _| Ok(pk_a));

    // Intrinsic gas placeholder expectations.
    const GAS_BASE_APPLY7702: i64 = 0;
    const GAS_PER_AUTH_TUPLE: i64 = 10_000;
    rt.expect_gas_charge(GAS_BASE_APPLY7702);
    rt.expect_gas_charge(GAS_PER_AUTH_TUPLE);

    // Build ApplyAndCall with a single tuple (A -> B) and call A (EOA) to trigger InvokeAsEoa.
    let list = vec![evm::DelegationParam {
        chain_id: 0,
        address: b_eth,
        nonce: 0,
        y_parity: 0,
        r: vec![1u8; 32],
        s: vec![1u8; 32],
    }];
    let params = evm::ApplyAndCallParams { list, call: evm::ApplyCall { to: a_eth, value: vec![], input: vec![] } };

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

    // Expect InvokeAsEoa to fail (non-success exit code) and include some revert data.
    let revert_data = vec![0xAA, 0xBB];
    rt.expect_send_any_params(
        rt.receiver,
        evm::Method::InvokeAsEoa as u64,
        TokenAmount::from_whole(0),
        None,
        SendFlags::default(),
        SendOutcome {
            send_return: Some(IpldBlock { codec: IPLD_RAW, data: revert_data.clone() }),
            exit_code: ExitCode::USR_ILLEGAL_STATE,
            send_error: None,
        },
    );

    // ApplyAndCall must succeed (ExitCode::OK) and embed status=0 even though InvokeAsEoa failed.
    rt.expect_validate_caller_any();
    let res = rt.call::<evm::EvmContractActor>(
        evm::Method::ApplyAndCall as u64,
        IpldBlock::serialize_dag_cbor(&params).unwrap(),
    );

    // Verify that the expected sends (GetBytecode, InvokeAsEoa) occurred even though the call failed.
    // This makes it explicit we exercised the delegated EOA path up to the point of failure.
    rt.verify();

    // With the current buggy implementation, this is Err(...) due to `?` propagation. Surface
    // the exit code for clarity.
    if let Err(err) = &res {
        panic!(
            "ApplyAndCall aborted on InvokeAsEoa failure; state was rolled back (exit_code={:?})",
            err.exit_code()
        );
    }

    // Decode ApplyAndCallReturn and assert status=0 (outer call failure).
    #[derive(fvm_ipld_encoding::serde::Deserialize)]
    struct ApplyAndCallReturn { status: u8, #[allow(dead_code)] output_data: Vec<u8> }
    let ret = res.unwrap().unwrap();
    let ApplyAndCallReturn { status, output_data } = ret
        .deserialize::<ApplyAndCallReturn>()
        .unwrap_or_else(|_| fvm_ipld_encoding::from_slice::<ApplyAndCallReturn>(&ret.data).unwrap());
    assert_eq!(status, 0, "expected embedded failure status from outer call");
    assert_eq!(output_data, revert_data, "expected revert data passthrough");

    // Additionally, verify nonce bump persisted by attempting to re-apply with nonce=0 and expecting a mismatch.
    let list_nonce0 = vec![evm::DelegationParam {
        chain_id: 0,
        address: b_eth,
        nonce: 0, // reusing 0 should now fail after the first application
        y_parity: 0,
        r: vec![2u8; 32],
        s: vec![2u8; 32],
    }];
    let params_again = evm::ApplyAndCallParams { list: list_nonce0, call: evm::ApplyCall { to: a_eth, value: vec![], input: vec![] } };
    // Gas for second attempt
    rt.expect_gas_charge(GAS_BASE_APPLY7702);
    rt.expect_gas_charge(GAS_PER_AUTH_TUPLE);
    rt.expect_validate_caller_any();
    let res2 = rt.call::<evm::EvmContractActor>(
        evm::Method::ApplyAndCall as u64,
        IpldBlock::serialize_dag_cbor(&params_again).unwrap(),
    );
    assert!(res2.is_err(), "re-applying nonce=0 should fail after first apply");
    assert_eq!(res2.err().unwrap().exit_code(), ExitCode::USR_ILLEGAL_ARGUMENT);
}
