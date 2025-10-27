use cid::Cid;
use fil_actor_evm as evm;
use fil_actors_evm_shared::address::EthAddress;
use fil_actors_runtime::test_utils::SendOutcome;
use fvm_ipld_blockstore::Blockstore;
use fvm_ipld_encoding::ipld_block::IpldBlock;
use fvm_shared::event::{ActorEvent, Entry, Flags};
use fvm_shared::{
    IPLD_RAW, address::Address as FilAddress, econ::TokenAmount, error::ExitCode, sys::SendFlags,
};

mod util;

fn put_code(rt: &fil_actors_runtime::test_utils::MockRuntime, code: &[u8]) -> Cid {
    use multihash_codetable::Code;
    rt.store
        .put(Code::Blake2b256, &fvm_ipld_blockstore::Block::new(IPLD_RAW, code))
        .expect("put code")
}

// Delegated SELFDESTRUCT under InvokeAsEoa must be a no-op for the EVM actor: no fund transfer,
// no tombstone, success status, and delegated event emitted.
#[test]
fn apply_and_call_delegated_selfdestruct_is_noop() {
    // Prepare runtime with empty code EVM actor.
    let mut rt = util::construct_and_verify(vec![]);

    // Build delegate bytecode that simply SELFDESTRUCTs to an arbitrary beneficiary.
    // PUSH20 <beneficiary> ; SELFDESTRUCT ; STOP
    let beneficiary = EthAddress::from_id(4242);
    let mut code: Vec<u8> = Vec::new();
    code.push(0x73); // PUSH20
    code.extend_from_slice(beneficiary.as_ref());
    code.push(0xff); // SELFDESTRUCT
    code.push(0x00); // STOP
    let code_cid = put_code(&rt, &code);

    // Choose a fixed public key for authority recovery used in ApplyAndCall.
    let mut pk_a = [0u8; 65];
    pk_a[0] = 0x04;
    for b in pk_a.iter_mut().skip(1) {
        *b = 0xA1;
    }
    // Derive EthAddress for authority A.
    use fil_actors_runtime::test_utils::hash as rt_hash;
    use fvm_shared::crypto::hash::SupportedHashes;
    let (keccak_a, _) = rt_hash(SupportedHashes::Keccak256, &pk_a[1..]);
    let mut a20 = [0u8; 20];
    a20.copy_from_slice(&keccak_a[12..32]);
    let a_eth = EthAddress(a20);

    // Map A -> B where B is the receiver EVM actor (ID 0) with known ETH f4 address.
    let b_eth: EthAddress = EthAddress(util::CONTRACT_ADDRESS);

    // Make signature recovery deterministic to A.
    rt.recover_secp_pubkey_fn = Box::new(move |_, _| Ok(pk_a));

    const GAS_BASE_APPLY7702: i64 = 0;
    const GAS_PER_AUTH_TUPLE: i64 = 10_000;
    rt.expect_gas_charge(GAS_BASE_APPLY7702);
    rt.expect_gas_charge(GAS_PER_AUTH_TUPLE);

    // Expect exactly two sends: GetBytecode(delegate=B) and InvokeAsEoa; no fund transfer from SELFDESTRUCT.
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
    rt.expect_send_any_params(
        rt.receiver,
        evm::Method::InvokeAsEoa as u64,
        TokenAmount::from_whole(0),
        None,
        SendFlags::default(),
        SendOutcome {
            send_return: Some(IpldBlock { codec: IPLD_RAW, data: Vec::new() }),
            exit_code: ExitCode::OK,
            send_error: None,
        },
    );

    // Expect the synthetic delegated event.
    let (topic, len) = rt_hash(SupportedHashes::Keccak256, b"EIP7702Delegated(address)");
    rt.expect_emitted_event(ActorEvent {
        entries: vec![
            Entry {
                flags: Flags::FLAG_INDEXED_ALL,
                key: "t1".to_owned(),
                codec: IPLD_RAW,
                value: topic[..len].to_vec(),
            },
            Entry {
                flags: Flags::FLAG_INDEXED_ALL,
                key: "d".to_owned(),
                codec: IPLD_RAW,
                value: b_eth.as_ref().to_vec(),
            },
        ],
    });

    // Build ApplyAndCall with single tuple A->B and call A.
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

    rt.expect_validate_caller_any();
    let res = rt.call::<evm::EvmContractActor>(
        evm::Method::ApplyAndCall as u64,
        IpldBlock::serialize_dag_cbor(&params).unwrap(),
    );
    assert!(res.is_ok());

    // Verify expectations and ensure no tombstone was set on the EVM actor.
    rt.verify();
    let state: evm::State = rt.get_state();
    assert!(state.tombstone.is_none());
}

// Variant: outer call includes a non-zero value transfer. Delegated SELFDESTRUCT remains a no-op
// (no transfer to beneficiary, no tombstone). Expect exactly three sends overall:
// 1) GetBytecode(delegate=B), 2) METHOD_SEND value transfer to authority (EOA), and
// 3) InvokeAsEoa.
#[test]
fn apply_and_call_delegated_selfdestruct_with_value_noop() {
    let mut rt = util::construct_and_verify(vec![]);

    // Build delegate code that SELFDESTRUCTs to an arbitrary beneficiary.
    let beneficiary = EthAddress::from_id(5151);
    let mut code: Vec<u8> = Vec::new();
    code.push(0x73); // PUSH20
    code.extend_from_slice(beneficiary.as_ref());
    code.push(0xff); // SELFDESTRUCT
    code.push(0x00); // STOP
    let code_cid = put_code(&rt, &code);

    // Authority A
    let mut pk_a = [0u8; 65];
    pk_a[0] = 0x04;
    for b in pk_a.iter_mut().skip(1) {
        *b = 0xB2;
    }
    rt.recover_secp_pubkey_fn = Box::new(move |_, _| Ok(pk_a));
    use fil_actors_runtime::test_utils::hash as rt_hash;
    use fvm_shared::crypto::hash::SupportedHashes;
    let (keccak_a, _) = rt_hash(SupportedHashes::Keccak256, &pk_a[1..]);
    let mut a20 = [0u8; 20];
    a20.copy_from_slice(&keccak_a[12..32]);
    let a_eth = EthAddress(a20);

    // Delegate B is receiver EVM actor
    let b_eth: EthAddress = EthAddress(util::CONTRACT_ADDRESS);

    const GAS_BASE_APPLY7702: i64 = 0;
    const GAS_PER_AUTH_TUPLE: i64 = 10_000;
    rt.expect_gas_charge(GAS_BASE_APPLY7702);
    rt.expect_gas_charge(GAS_PER_AUTH_TUPLE);

    // Expect 1) GetBytecode
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

    // Expect 2) value transfer to authority (EOA) before InvokeAsEoa.
    let authority_f4: FilAddress = a_eth.into();
    let send_value = TokenAmount::from_atto(1234u64);
    rt.expect_send_simple(
        authority_f4,
        fvm_shared::METHOD_SEND,
        None,
        send_value.clone(),
        None,
        ExitCode::OK,
    );

    // Expect 3) InvokeAsEoa call.
    rt.expect_send_any_params(
        rt.receiver,
        evm::Method::InvokeAsEoa as u64,
        TokenAmount::from_whole(0),
        None,
        SendFlags::default(),
        SendOutcome {
            send_return: Some(IpldBlock { codec: IPLD_RAW, data: Vec::new() }),
            exit_code: ExitCode::OK,
            send_error: None,
        },
    );

    // Expect delegated event emission.
    let (topic, len) = rt_hash(SupportedHashes::Keccak256, b"EIP7702Delegated(address)");
    rt.expect_emitted_event(ActorEvent {
        entries: vec![
            Entry {
                flags: Flags::FLAG_INDEXED_ALL,
                key: "t1".to_owned(),
                codec: IPLD_RAW,
                value: topic[..len].to_vec(),
            },
            Entry {
                flags: Flags::FLAG_INDEXED_ALL,
                key: "d".to_owned(),
                codec: IPLD_RAW,
                value: b_eth.as_ref().to_vec(),
            },
        ],
    });

    // Apply A->B and call A with non-zero value.
    let list = vec![evm::DelegationParam {
        chain_id: 0,
        address: b_eth,
        nonce: 0,
        y_parity: 0,
        r: vec![1u8; 32],
        s: vec![1u8; 32],
    }];
    // value encoded as big-endian magnitude; 1234 atto
    let call_value = vec![0x04, 0xD2];
    let params = evm::ApplyAndCallParams {
        list,
        call: evm::ApplyCall { to: a_eth, value: call_value, input: vec![] },
    };

    rt.expect_validate_caller_any();
    let res = rt.call::<evm::EvmContractActor>(
        evm::Method::ApplyAndCall as u64,
        IpldBlock::serialize_dag_cbor(&params).unwrap(),
    );
    assert!(res.is_ok());
    rt.verify();

    // Ensure actor not tombstoned.
    let state: evm::State = rt.get_state();
    assert!(state.tombstone.is_none());
}
