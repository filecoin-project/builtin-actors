use fil_actor_ethaccount as ethaccount;
use fil_actor_ethaccount::state::State;
use fil_actors_evm_shared::address::EthAddress;
use fil_actors_evm_shared::eip7702;
use fil_actors_runtime::EAM_ACTOR_ID;
use fil_actors_runtime::test_utils::{EVM_ACTOR_CODE_ID, MockRuntime, SendOutcome};
use fvm_ipld_encoding::ipld_block::IpldBlock;
use fvm_shared::address::Address;
use fvm_shared::econ::TokenAmount;
use fvm_shared::error::ExitCode;
use fvm_shared::sys::SendFlags;

/// When the outer call target resolves to an EVM contract actor, EthAccount.ApplyAndCall
/// should route via the EVM InvokeContract entrypoint instead of a plain METHOD_SEND, and
/// embed the callee's status/returndata in the ApplyAndCallReturn.
#[test]
fn outer_call_routes_through_evm_invoke_contract() {
    let mut rt = MockRuntime::new();
    rt.expect_validate_caller_any();

    // Receiver EthAccount EOA: derive a stable 20-byte address from a fixed pubkey.
    let mut pk = [0u8; 65];
    pk[0] = 0x04;
    for b in pk.iter_mut().skip(1) {
        *b = 0xA9;
    }
    let (keccak_a, _) = fil_actors_runtime::test_utils::hash(
        fvm_shared::crypto::hash::SupportedHashes::Keccak256,
        &pk[1..],
    );
    let mut a20 = [0u8; 20];
    a20.copy_from_slice(&keccak_a[12..32]);

    // EthAccount actor lives at ID 1000 with an f4 delegated address.
    let ethaccount_id = 1000;
    let eth_f4 = Address::new_delegated(EAM_ACTOR_ID, &a20).unwrap();
    rt.set_delegated_address(ethaccount_id, eth_f4);
    rt.caller.replace(Address::new_id(10));
    rt.receiver = Address::new_id(ethaccount_id);
    rt.recover_secp_pubkey_fn = Box::new(move |_, _| Ok(pk));

    // EVM contract actor at ID 2000 with delegated f4 address derived from a fixed 20-byte eth address.
    let evm_eth = EthAddress([0xAB; 20]);
    let evm_f4 = Address::new_delegated(EAM_ACTOR_ID, evm_eth.as_ref()).unwrap();
    let evm_id = 2000;
    rt.set_delegated_address(evm_id, evm_f4);
    rt.set_address_actor_type(Address::new_id(evm_id), *EVM_ACTOR_CODE_ID);

    // Single valid delegation tuple targeting the receiver authority.
    let delegate = EthAddress([9u8; 20]);
    let list = vec![eip7702::DelegationParam {
        chain_id: 0,
        address: delegate,
        nonce: 0,
        y_parity: 0,
        r: vec![1u8; 32],
        s: vec![1u8; 32],
    }];

    // Outer call targets the EVM contract with zero value and some calldata.
    let calldata = vec![0xDE, 0xAD, 0xBE, 0xEF];
    let call = eip7702::ApplyCall { to: evm_eth, value: vec![], input: calldata.clone() };
    let params = eip7702::ApplyAndCallParams { list, call };

    // Expect a send to the EVM actor using the InvokeEVM selector, with zero value and
    // default flags. We don't assert the encoded params shape here (wildcard match).
    let method_invoke_evm = frc42_dispatch::method_hash!("InvokeEVM");
    let expected_return_bytes = vec![0xAA, 0xBB, 0xCC];
    rt.expect_send_any_params(
        evm_f4,
        method_invoke_evm,
        TokenAmount::from_atto(0u8),
        None,
        SendFlags::default(),
        SendOutcome {
            send_return: Some(IpldBlock {
                codec: fvm_ipld_encoding::CBOR,
                data: expected_return_bytes.clone(),
            }),
            exit_code: ExitCode::OK,
            send_error: None,
        },
    );

    let res = rt.call::<ethaccount::EthAccountActor>(
        ethaccount::Method::ApplyAndCall as u64,
        IpldBlock::serialize_dag_cbor(&params).unwrap(),
    );
    assert!(res.is_ok(), "ApplyAndCall should succeed with embedded status");

    let state: State = rt.get_state();
    assert_eq!(state.delegate_to, Some(delegate));
    assert_eq!(state.auth_nonce, 1);

    // Decode the embedded ApplyAndCallReturn and verify status/output_data.
    let ret_blk = res.unwrap().expect("expected non-empty ApplyAndCall return");
    let ret: eip7702::ApplyAndCallReturn =
        fvm_ipld_encoding::from_slice(&ret_blk.data).expect("failed to decode ApplyAndCallReturn");
    assert_eq!(ret.status, 1, "expected status=1 for ExitCode::OK");
    assert_eq!(ret.output_data, expected_return_bytes);
}
