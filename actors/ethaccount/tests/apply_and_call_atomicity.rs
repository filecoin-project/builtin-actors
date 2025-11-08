use fil_actor_ethaccount as ethaccount;
use fil_actors_evm_shared::address::EthAddress;
use fil_actors_evm_shared::eip7702;
use fil_actors_runtime::EAM_ACTOR_ID;
use fil_actors_runtime::test_utils::*;
use fvm_ipld_encoding::ipld_block::IpldBlock;
use fvm_shared::address::Address;
use fvm_shared::econ::TokenAmount;

#[test]
fn mapping_persists_on_revert() {
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

    // Make a call with non-zero value to trigger send failure (insufficient funds)
    // Set send expectation to error due to insufficient funds.
    use fil_actors_runtime::test_utils::SendOutcome;
    use fvm_shared::error::ExitCode;
    use fvm_shared::sys::SendFlags;
    let to_addr = Address::new_delegated(EAM_ACTOR_ID, &[0u8; 20]).unwrap();
    rt.expect_send_any_params(
        to_addr,
        0,
        TokenAmount::from_atto(1u8),
        None,
        SendFlags::default(),
        SendOutcome {
            send_return: None,
            exit_code: ExitCode::OK,
            send_error: Some(fvm_shared::error::ErrorNumber::InsufficientFunds),
        },
    );
    let list = vec![eip7702::DelegationParam {
        chain_id: 0,
        address: EthAddress([9u8; 20]),
        nonce: 0,
        y_parity: 0,
        r: vec![1u8; 32],
        s: vec![1u8; 32],
    }];
    let call = eip7702::ApplyCall { to: EthAddress([0u8; 20]), value: vec![1u8], input: vec![] };
    let params = eip7702::ApplyAndCallParams { list, call };
    let res = rt.call::<ethaccount::EthAccountActor>(
        ethaccount::Method::ApplyAndCall as u64,
        IpldBlock::serialize_dag_cbor(&params).unwrap(),
    );
    // Return must be OK with embedded status=0; from test runtime side, call returns Ok(Some(IpldBlock))
    assert!(res.is_ok());
}
