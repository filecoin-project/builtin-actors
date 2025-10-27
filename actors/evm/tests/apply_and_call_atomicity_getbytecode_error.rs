use fil_actor_evm as evm;
use fil_actors_evm_shared::address::EthAddress;
use fvm_ipld_encoding::ipld_block::IpldBlock;
use fvm_shared::{
    address::Address as FilAddress, econ::TokenAmount, error::ExitCode, sys::SendFlags,
};

mod util;

// Verify that a failure when fetching the delegate's bytecode (GetBytecode) does not abort
// ApplyAndCall after state has been flushed. ApplyAndCall must return ExitCode::OK with
// embedded status=0, and the nonce bump must persist.
#[test]
fn apply_and_call_getbytecode_error_keeps_state_and_returns_ok() {
    let mut rt = util::construct_and_verify(vec![]);

    // Prepare a deterministic recovered authority A and delegate B (the constructed EVM actor).
    let mut pk_a = [0u8; 65];
    pk_a[0] = 0x04;
    for b in pk_a.iter_mut().skip(1) {
        *b = 0xA1;
    }
    // Derive B's EthAddress from util constant.
    let b_eth: EthAddress = EthAddress(util::CONTRACT_ADDRESS);

    // Force signature recovery to return A.
    rt.recover_secp_pubkey_fn = Box::new(move |_, _| Ok(pk_a));

    // Gas charges
    const GAS_BASE_APPLY7702: i64 = 0;
    const GAS_PER_AUTH_TUPLE: i64 = 10_000;
    rt.expect_gas_charge(GAS_BASE_APPLY7702);
    rt.expect_gas_charge(GAS_PER_AUTH_TUPLE);

    // Outer call targets an EOA (authority A) to exercise the InvokeAsEoa path, but we will make
    // GetBytecode(delegate=B) fail.
    // Compute A's EthAddress as the actor would: keccak(pubkey[1:])[12:]
    use fil_actors_runtime::test_utils::hash as rt_hash;
    use fvm_shared::crypto::hash::SupportedHashes;
    let (keccak_a, _) = rt_hash(SupportedHashes::Keccak256, &pk_a[1..]);
    let mut a20 = [0u8; 20];
    a20.copy_from_slice(&keccak_a[12..32]);
    let a_eth = EthAddress(a20);

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

    // Make GetBytecode return an error exit code (simulating a failure to fetch code).
    use fil_actors_runtime::test_utils::EVM_ACTOR_CODE_ID;
    let b_f4: FilAddress = b_eth.into();
    let b_id = FilAddress::new_id(0x444u64);
    rt.set_delegated_address(b_id.id().unwrap(), b_f4);
    rt.actor_code_cids.borrow_mut().insert(b_id, *EVM_ACTOR_CODE_ID);
    rt.expect_send(
        b_id,
        evm::Method::GetBytecode as u64,
        None,
        TokenAmount::from_whole(0),
        None,
        SendFlags::READ_ONLY,
        None,
        ExitCode::USR_ILLEGAL_STATE,
        None,
    );

    // ApplyAndCall must not abort even though GetBytecode failed.
    rt.expect_validate_caller_any();
    let res = rt.call::<evm::EvmContractActor>(
        evm::Method::ApplyAndCall as u64,
        IpldBlock::serialize_dag_cbor(&params).unwrap(),
    );
    assert!(res.is_ok(), "ApplyAndCall aborted on GetBytecode failure");

    // Now attempt to re-apply with nonce=0 and expect a mismatch (nonce was bumped by first call).
    let params_again = evm::ApplyAndCallParams {
        list: vec![evm::DelegationParam {
            chain_id: 0,
            address: b_eth,
            nonce: 0, // should now be invalid
            y_parity: 0,
            r: vec![2u8; 32],
            s: vec![2u8; 32],
        }],
        call: evm::ApplyCall { to: a_eth, value: vec![], input: vec![] },
    };

    // Gas charges for second attempt
    rt.expect_gas_charge(GAS_BASE_APPLY7702);
    rt.expect_gas_charge(GAS_PER_AUTH_TUPLE);
    rt.expect_validate_caller_any();
    let res2 = rt.call::<evm::EvmContractActor>(
        evm::Method::ApplyAndCall as u64,
        IpldBlock::serialize_dag_cbor(&params_again).unwrap(),
    );
    assert!(res2.is_err(), "re-applying nonce=0 should fail after first apply");
}
