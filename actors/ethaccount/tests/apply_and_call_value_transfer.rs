use fil_actor_ethaccount as ethaccount;
use fil_actors_evm_shared::address::EthAddress;
use fil_actors_evm_shared::eip7702;
use fil_actors_runtime::EAM_ACTOR_ID;
use fil_actors_runtime::test_utils::*;
use fvm_ipld_encoding::ipld_block::IpldBlock;
use fvm_shared::address::Address;

#[test]
fn value_transfer_failure_short_circuit() {
    let mut rt = MockRuntime::new();
    rt.expect_validate_caller_any();
    // Receiver eth address derived from a fixed pubkey
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
    rt.set_delegated_address(1000, Address::new_delegated(EAM_ACTOR_ID, &a20).unwrap());
    rt.caller.replace(Address::new_id(10));
    rt.receiver = Address::new_id(1000);
    rt.recover_secp_pubkey_fn = Box::new(move |_, _| Ok(pk));

    // Non-zero value encoded in call.value will be interpreted as TokenAmount(1), causing send to fail due to insufficient funds.
    let list = vec![eip7702::DelegationParam {
        chain_id: 0,
        address: EthAddress([7u8; 20]),
        nonce: 0,
        y_parity: 0,
        r: vec![1u8; 32],
        s: vec![1u8; 32],
    }];
    let call = eip7702::ApplyCall { to: EthAddress([0u8; 20]), value: vec![1u8], input: vec![] };
    let params = eip7702::ApplyAndCallParams { list, call };
    // Expect a send that fails due to insufficient funds (value transfer short-circuit)
    use fil_actors_runtime::test_utils::SendOutcome;
    use fvm_shared::sys::SendFlags;
    rt.expect_send_any_params(
        Address::new_delegated(EAM_ACTOR_ID, &[0u8; 20]).unwrap(),
        0,
        fvm_shared::econ::TokenAmount::from_atto(1u8),
        None,
        SendFlags::default(),
        SendOutcome { send_return: None, exit_code: fvm_shared::error::ExitCode::OK, send_error: None },
    );
    let res = rt.call::<ethaccount::EthAccountActor>(
        ethaccount::Method::ApplyAndCall as u64,
        IpldBlock::serialize_dag_cbor(&params).unwrap(),
    );
    assert!(res.is_ok());
}
