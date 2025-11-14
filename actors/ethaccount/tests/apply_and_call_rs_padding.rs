use fil_actor_ethaccount as ethaccount;
use fil_actors_evm_shared::{address::EthAddress, eip7702};
use fil_actors_runtime::EAM_ACTOR_ID;
use fil_actors_runtime::test_utils::{MockRuntime, SendOutcome, hash};
use fvm_ipld_encoding::ipld_block::IpldBlock;
use fvm_shared::address::Address;
use fvm_shared::econ::TokenAmount;
use fvm_shared::error::ExitCode;
use fvm_shared::sys::SendFlags;

/// EthAccount must accept minimally-encoded big-endian r/s values with
/// lengths 1..=32 bytes (left-padded internally), rejecting only >32-byte
/// or all-zero values. This test exercises the positive paths for
/// representative lengths {1, 31, 32}.
#[test]
fn accepts_minimally_encoded_rs_lengths() {
    let mut rt = MockRuntime::new();
    rt.expect_validate_caller_any();

    // Receiver eth address derived from a fixed pubkey; the mock runtime
    // always recovers this key, so any r/s that pass local validation
    // should be accepted.
    let mut pk = [0u8; 65];
    pk[0] = 0x04;
    for b in pk.iter_mut().skip(1) {
        *b = 0xA9;
    }
    let (keccak_a, _) = hash(fvm_shared::crypto::hash::SupportedHashes::Keccak256, &pk[1..]);
    let mut a20 = [0u8; 20];
    a20.copy_from_slice(&keccak_a[12..32]);
    let ethaccount_id = 1000;
    let eth_f4 = Address::new_delegated(EAM_ACTOR_ID, &a20).unwrap();
    rt.set_delegated_address(ethaccount_id, eth_f4);
    rt.caller.replace(Address::new_id(10));
    rt.receiver = Address::new_id(ethaccount_id);
    rt.recover_secp_pubkey_fn = Box::new(move |_, _| Ok(pk));

    // Outer call: zero value to a non-EVM target; we only care that the
    // send succeeds so ApplyAndCall returns OK from the actor perspective.
    let call_to = EthAddress([0u8; 20]);
    let call = eip7702::ApplyCall { to: call_to, value: vec![], input: vec![] };

    // Exercise r/s lengths {1, 31, 32}; non-zero values ensure we stay on
    // the positive side of the zero/length/low-S checks.
    let lengths = [1usize, 31, 32];
    for (idx, len) in lengths.iter().enumerate() {
        let nonce = idx as u64;
        let list = vec![eip7702::DelegationParam {
            chain_id: 0,
            address: EthAddress([9u8; 20]),
            nonce,
            y_parity: 0,
            r: vec![1u8; *len],
            s: vec![1u8; *len],
        }];
        let params = eip7702::ApplyAndCallParams { list, call: call.clone() };

        // Each ApplyAndCall invocation validates the caller and performs a
        // send; arm expectations for this iteration.
        rt.expect_validate_caller_any();
        rt.expect_send_any_params(
            Address::new_delegated(EAM_ACTOR_ID, call_to.as_ref()).unwrap(),
            0,
            TokenAmount::from_atto(0u8),
            None,
            SendFlags::default(),
            SendOutcome { send_return: None, exit_code: ExitCode::OK, send_error: None },
        );

        let res = rt.call::<ethaccount::EthAccountActor>(
            ethaccount::Method::ApplyAndCall as u64,
            IpldBlock::serialize_dag_cbor(&params).unwrap(),
        );
        assert!(res.is_ok(), "ApplyAndCall should accept r/s length {}", len);
    }
}
