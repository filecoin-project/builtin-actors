use cid::Cid;
use fil_actor_evm as evm;
use fil_actors_evm_shared::address::EthAddress;
use fil_actors_runtime::test_utils::EVM_ACTOR_CODE_ID;
use fvm_ipld_blockstore::Blockstore;
use fil_actors_runtime::runtime::Primitives;
use fvm_ipld_encoding::ipld_block::IpldBlock;
use fvm_shared::address::Address as FilAddress;
use fvm_shared::econ::TokenAmount;
use fvm_shared::error::ExitCode;
use fvm_shared::event::{ActorEvent, Entry, Flags};
use fvm_shared::sys::SendFlags;
use fvm_shared::version::NetworkVersion;
use fvm_shared::IPLD_RAW;

mod util;
mod asm;

// Outer contract that CALLs a destination EOA; it forwards returndata.
fn outer_call_proxy() -> Vec<u8> {
    let init = "";
    let body = r#"
push1 0x20
calldatasize
sub
push1 0x20
push1 0x00
calldatacopy
push2 0x00
push1 0x00
push1 0x20
calldatasize
sub
push1 0x00
push1 0x00
push1 0x00
calldataload
push4 0xffffffff
call
returndatasize
push1 0x00
push1 0x00
returndatacopy
returndatasize
push1 0x00
return
"#;
    asm::new_contract("outer-call-proxy", init, body).unwrap()
}

// Delegate bytecode that issues a CALL using its calldata's 32-byte address parameter.
fn delegate_call_proxy() -> Vec<u8> { outer_call_proxy() }

#[test]
#[ignore]
fn nested_eoa_delegation_two_layers() {
    // Construct an outer contract that CALLs destination and returns returndata.
    let initcode = outer_call_proxy();
    let rt = util::construct_and_verify(initcode);
    rt.set_network_version(NetworkVersion::V16);

    // EOA A and EOA B, both mapped to delegates.
    let authority_a = EthAddress(hex_literal::hex!("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"));
    let authority_b = EthAddress(hex_literal::hex!("bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"));
    let authority_a_word = authority_a.as_evm_word();

    // delegate1 for A (calls B via its calldata), delegate2 for B.
    let delegate1_eth = EthAddress(hex_literal::hex!("1111111111111111111111111111111111111111"));
    let delegate1_id = FilAddress::new_id(0x111);
    rt.set_delegated_address(delegate1_id.id().unwrap(), delegate1_eth.into());
    rt.actor_code_cids.borrow_mut().insert(delegate1_id, *EVM_ACTOR_CODE_ID);

    let delegate2_eth = EthAddress(hex_literal::hex!("2222222222222222222222222222222222222222"));
    let delegate2_id = FilAddress::new_id(0x222);
    rt.set_delegated_address(delegate2_id.id().unwrap(), delegate2_eth.into());
    rt.actor_code_cids.borrow_mut().insert(delegate2_id, *EVM_ACTOR_CODE_ID);

    // Store delegate1 bytecode (call proxy) and delegate2 bytecode (STOP).
    let bytecode1_cid = Cid::try_from("baeaikaia").unwrap();
    let code1 = delegate_call_proxy();
    rt.store.put_keyed(&bytecode1_cid, &code1).unwrap();

    // First lookup: A -> delegate1
    #[derive(fvm_ipld_encoding::serde::Serialize, fvm_ipld_encoding::serde::Deserialize)]
    struct LookupDelegateParams { authority: EthAddress }
    #[derive(fvm_ipld_encoding::serde::Serialize, fvm_ipld_encoding::serde::Deserialize)]
    struct LookupDelegateReturn { delegate: Option<EthAddress> }
    rt.expect_send(
        fil_actors_runtime::DELEGATOR_ACTOR_ADDR,
        frc42_dispatch::method_hash!("LookupDelegate"),
        IpldBlock::serialize_cbor(&LookupDelegateParams { authority: authority_a }).unwrap(),
        TokenAmount::from_whole(0),
        None,
        SendFlags::READ_ONLY,
        IpldBlock::serialize_cbor(&LookupDelegateReturn { delegate: Some(delegate1_eth) }).unwrap(),
        ExitCode::OK,
        None,
    );
    // Get bytecode for delegate1
    rt.expect_send(
        delegate1_id,
        evm::Method::GetBytecode as u64,
        None,
        TokenAmount::from_whole(0),
        None,
        SendFlags::READ_ONLY,
        IpldBlock::serialize_cbor(&Some(bytecode1_cid)).unwrap(),
        ExitCode::OK,
        None,
    );

    // Gas query for the first InvokeAsEoa
    rt.expect_gas_available(10_000_000_000u64);

    // Expect the first delegated execution event (delegate1).
    let topic = rt.hash(fvm_shared::crypto::hash::SupportedHashes::Keccak256, b"EIP7702Delegated(address)");
    rt.expect_emitted_event(ActorEvent::from(vec![
        Entry { flags: Flags::FLAG_INDEXED_ALL, key: "t1".to_owned(), codec: IPLD_RAW, value: topic.clone() },
        Entry { flags: Flags::FLAG_INDEXED_ALL, key: "d".to_owned(), codec: IPLD_RAW, value: delegate1_eth.as_ref().to_vec() },
    ]));
    // Mock the first InvokeAsEoa call returning empty data.
    rt.expect_send_any_params(
        rt.receiver,
        evm::Method::InvokeAsEoa as u64,
        TokenAmount::from_whole(0),
        Some(0xffff_ffff),
        SendFlags::empty(),
        Some(IpldBlock { codec: IPLD_RAW, data: vec![] }),
        ExitCode::OK,
        None,
    );

    // Nested path: within delegate1 code, a CALL to B; expect lookup + getbytecode for B
    rt.expect_send(
        fil_actors_runtime::DELEGATOR_ACTOR_ADDR,
        frc42_dispatch::method_hash!("LookupDelegate"),
        IpldBlock::serialize_cbor(&LookupDelegateParams { authority: authority_b }).unwrap(),
        TokenAmount::from_whole(0),
        None,
        SendFlags::READ_ONLY,
        IpldBlock::serialize_cbor(&LookupDelegateReturn { delegate: Some(delegate2_eth) }).unwrap(),
        ExitCode::OK,
        None,
    );
    rt.expect_send(
        delegate2_id,
        evm::Method::GetBytecode as u64,
        None,
        TokenAmount::from_whole(0),
        None,
        SendFlags::READ_ONLY,
        IpldBlock::serialize_cbor(&Some(bytecode1_cid)).unwrap(),
        ExitCode::OK,
        None,
    );

    // Expect event for delegate2 and the nested InvokeAsEoa to self.
    rt.expect_emitted_event(ActorEvent::from(vec![
        Entry { flags: Flags::FLAG_INDEXED_ALL, key: "t1".to_owned(), codec: IPLD_RAW, value: topic.clone() },
        Entry { flags: Flags::FLAG_INDEXED_ALL, key: "d".to_owned(), codec: IPLD_RAW, value: delegate2_eth.as_ref().to_vec() },
    ]));
    rt.expect_send_any_params(
        rt.receiver,
        evm::Method::InvokeAsEoa as u64,
        TokenAmount::from_whole(0),
        Some(0xffff_ffff),
        SendFlags::empty(),
        Some(IpldBlock { codec: IPLD_RAW, data: vec![] }),
        ExitCode::OK,
        None,
    );

    // Build outer call params: the input to the outer proxy contains authority A as destination,
    // and its calldata (after the first 32 bytes) is not used by our simple proxies, so to direct
    // the nested call to B, we provide B's address as the first 32 bytes of calldata passed into
    // delegate1 (i.e., we reuse the same proxy shape at both layers). Here we just feed B to the
    // outer proxy; delegate1 will read that as its target.
    let mut call_params = vec![0u8; 32];
    authority_a_word.write_as_big_endian(&mut call_params[..]);
    let _ = util::invoke_contract(&rt, &call_params);
}
