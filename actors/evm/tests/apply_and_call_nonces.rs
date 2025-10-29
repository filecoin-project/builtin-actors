use fil_actor_evm as evm;
use fil_actors_evm_shared::address::EthAddress;
use fvm_ipld_encoding::ipld_block::IpldBlock;
use fvm_shared::error::ExitCode;

mod util;

// First-time authority should have nonce=0. Applying with nonce=0 should succeed and initialize
// the nonces map. Re-applying with nonce=0 for the same authority should be rejected.
#[test]
fn apply_and_call_nonce_initialization_and_repeat_rejection() {
    let mut rt = util::construct_and_verify(vec![]);

    // Make signature recovery deterministic and constant to control the recovered authority.
    let mut pk = [0u8; 65];
    pk[0] = 0x04; // uncompressed
    for b in pk.iter_mut().skip(1) {
        *b = 0xA5;
    }
    rt.recover_secp_pubkey_fn = Box::new(move |_, _| Ok(pk));


    // Build a single tuple at nonce=0, delegate to an arbitrary address, and call a different EOA
    // so no additional sends occur.
    let delegate = EthAddress::from_id(1001);
    let to_other = EthAddress::from_id(202);
    let list = vec![evm::DelegationParam {
        chain_id: 0,
        address: delegate,
        nonce: 0,
        y_parity: 0,
        r: vec![1u8; 32],
        s: vec![1u8; 32],
    }];
    let params_ok = evm::ApplyAndCallParams {
        list: list.clone(),
        call: evm::ApplyCall { to: to_other, value: vec![], input: vec![] },
    };

    // First application at nonce=0 succeeds and initializes nonce to 1.
    // No gas expectations in tests (behavioral only).
    rt.expect_validate_caller_any();
    let res = rt.call::<evm::EvmContractActor>(
        evm::Method::ApplyAndCall as u64,
        IpldBlock::serialize_dag_cbor(&params_ok).unwrap(),
    );
    assert!(res.is_ok());
    rt.verify();

    // Re-apply with the same authority at nonce=0 should now fail with illegal argument.
    // No gas expectations in tests (behavioral only).
    rt.expect_validate_caller_any();
    let res2 = rt.call::<evm::EvmContractActor>(
        evm::Method::ApplyAndCall as u64,
        IpldBlock::serialize_dag_cbor(&params_ok).unwrap(),
    );
    assert!(res2.is_err());
    assert_eq!(res2.err().unwrap().exit_code(), ExitCode::USR_ILLEGAL_ARGUMENT);

    // Optional: demonstrate that advancing to nonce=1 succeeds for the same authority.
    let list_nonce1 = vec![evm::DelegationParam {
        chain_id: 0,
        address: delegate,
        nonce: 1,
        y_parity: 0,
        r: vec![2u8; 32],
        s: vec![2u8; 32],
    }];
    let params_ok2 = evm::ApplyAndCallParams {
        list: list_nonce1,
        call: evm::ApplyCall { to: to_other, value: vec![], input: vec![] },
    };
    // No gas expectations in tests (behavioral only).
    rt.expect_validate_caller_any();
    let res3 = rt.call::<evm::EvmContractActor>(
        evm::Method::ApplyAndCall as u64,
        IpldBlock::serialize_dag_cbor(&params_ok2).unwrap(),
    );
    assert!(res3.is_ok());
}
