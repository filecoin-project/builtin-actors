use cid::Cid;
use fil_actor_evm as evm;
use fil_actors_evm_shared::address::EthAddress;
use fil_actors_runtime::test_utils::{EVM_ACTOR_CODE_ID, MockRuntime};
use fvm_ipld_blockstore::Blockstore;
use fvm_ipld_encoding::ipld_block::IpldBlock;
use fvm_shared::address::Address as FilAddress;
use fvm_shared::econ::TokenAmount;
use fvm_shared::error::ExitCode;
use fvm_shared::sys::SendFlags;
use fvm_shared::event::{ActorEvent, Entry, Flags};
use fvm_shared::crypto::hash::SupportedHashes;
use fvm_shared::version::NetworkVersion;
use fvm_shared::IPLD_RAW;

mod util;
mod call;

#[test]
fn call_to_eoa_uses_delegate_and_propagates_output() {
    // Construct a proxy contract that CALLs a destination and returns returndata.
    let initcode = call::call_proxy_contract();
    let rt = util::construct_and_verify(initcode);

    // Enable EIP-7702 path.
    rt.set_network_version(NetworkVersion::V16);

    // Destination is an EOA (no actor code registered, NotFound).
    let authority = EthAddress(hex_literal::hex!("00112233445566778899aabbccddeeff00112233"));
    let authority_word = authority.as_evm_word();

    // Choose a delegate EVM contract address and program runtime to resolve it as an EVM actor.
    let delegate_eth = EthAddress(hex_literal::hex!("feedfacecafebeef000000000000000000000000"));
    let delegate_f4: FilAddress = delegate_eth.into();
    let delegate_id = FilAddress::new_id(0x222u64);
    rt.set_delegated_address(delegate_id.id().unwrap(), delegate_f4);
    rt.actor_code_cids.borrow_mut().insert(delegate_id, *EVM_ACTOR_CODE_ID);

    // Store minimal delegate bytecode and return its CID from GetBytecode.
    // We'll craft a simple one-byte STOP; output will be mocked at InvokeAsEoa return anyway.
    let bytecode_cid = Cid::try_from("baeaikaia").unwrap();
    rt.store.put_keyed(&bytecode_cid, &[0x00]).unwrap();

    // Expect: Delegator LookupDelegate(authority) -> Some(delegate_eth)
    #[derive(fvm_ipld_encoding::serde::Serialize, fvm_ipld_encoding::serde::Deserialize)]
    struct LookupDelegateParams { authority: EthAddress }
    #[derive(fvm_ipld_encoding::serde::Serialize, fvm_ipld_encoding::serde::Deserialize)]
    struct LookupDelegateReturn { delegate: Option<EthAddress> }
    rt.expect_send(
        fil_actors_runtime::DELEGATOR_ACTOR_ADDR,
        frc42_dispatch::method_hash!("LookupDelegate"),
        IpldBlock::serialize_cbor(&LookupDelegateParams { authority }).unwrap(),
        TokenAmount::from_whole(0),
        None,
        SendFlags::READ_ONLY,
        IpldBlock::serialize_cbor(&LookupDelegateReturn { delegate: Some(delegate_eth) }).unwrap(),
        ExitCode::OK,
        None,
    );

    // Expect: GetBytecode(delegate_id) -> Some(bytecode_cid)
    rt.expect_send(
        delegate_id,
        evm::Method::GetBytecode as u64,
        None,
        TokenAmount::from_whole(0),
        None,
        SendFlags::READ_ONLY,
        IpldBlock::serialize_cbor(&Some(bytecode_cid)).unwrap(),
        ExitCode::OK,
        None,
    );

    // Expect gas query when computing call gas limit.
    rt.expect_gas_available(10_000_000_000u64);

    // Expect: internal self-call InvokeAsEoa with any params; return raw output bytes.
    let expected_output = vec![0xde, 0xad, 0xbe, 0xef];
    // Expect delegated execution marker event (topic0 + data=delegate 20b)
    let topic = rt.hash(SupportedHashes::Keccak256, b"EIP7702Delegated(address)");
    rt.expect_emitted_event(ActorEvent::from(vec![
        Entry { flags: Flags::FLAG_INDEXED_ALL, key: "t1".to_owned(), codec: IPLD_RAW, value: topic.clone() },
        Entry { flags: Flags::FLAG_INDEXED_ALL, key: "d".to_owned(), codec: IPLD_RAW, value: delegate_eth.as_ref().to_vec() },
    ]));
    rt.expect_send_any_params(
        rt.receiver,
        evm::Method::InvokeAsEoa as u64,
        TokenAmount::from_whole(0),
        Some(0xffff_ffff),
        SendFlags::empty(),
        Some(IpldBlock { codec: IPLD_RAW, data: expected_output.clone() }),
        ExitCode::OK,
        None,
    );

    // Build call parameters: [dest(32b)] with no additional payload.
    let mut call_params = vec![0u8; 32];
    authority_word.write_as_big_endian(&mut call_params[..]);

    // Invoke the contract and verify output propagates.
    let result = util::invoke_contract(&rt, &call_params);
    assert_eq!(result, expected_output);
}

#[test]
fn call_to_eoa_with_value_transfers_then_delegates() {
    // Construct a proxy contract that CALLs and forwards returndata, but sets non-zero value.
    let initcode = call::call_proxy_transfer_contract();
    let rt = util::construct_and_verify(initcode);

    // Enable EIP-7702 path.
    rt.set_network_version(NetworkVersion::V16);

    // Destination is an EOA (no actor code registered, NotFound).
    let authority = EthAddress(hex_literal::hex!("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"));
    let authority_word = authority.as_evm_word();
    let authority_f4: FilAddress = authority.into();

    // Delegate EVM
    let delegate_eth = EthAddress(hex_literal::hex!("bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"));
    let delegate_f4: FilAddress = delegate_eth.into();
    let delegate_id = FilAddress::new_id(0x333u64);
    rt.set_delegated_address(delegate_id.id().unwrap(), delegate_f4);
    rt.actor_code_cids.borrow_mut().insert(delegate_id, *EVM_ACTOR_CODE_ID);

    // Minimal bytecode for delegate contract
    let bytecode_cid = Cid::try_from("baeaikaia").unwrap();
    rt.store.put_keyed(&bytecode_cid, &[0x00]).unwrap();

    // Expect: LookupDelegate -> Some(delegate)
    #[derive(fvm_ipld_encoding::serde::Serialize, fvm_ipld_encoding::serde::Deserialize)]
    struct LookupDelegateParams { authority: EthAddress }
    #[derive(fvm_ipld_encoding::serde::Serialize, fvm_ipld_encoding::serde::Deserialize)]
    struct LookupDelegateReturn { delegate: Option<EthAddress> }
    rt.expect_send(
        fil_actors_runtime::DELEGATOR_ACTOR_ADDR,
        frc42_dispatch::method_hash!("LookupDelegate"),
        IpldBlock::serialize_cbor(&LookupDelegateParams { authority }).unwrap(),
        TokenAmount::from_whole(0),
        None,
        SendFlags::READ_ONLY,
        IpldBlock::serialize_cbor(&LookupDelegateReturn { delegate: Some(delegate_eth) }).unwrap(),
        ExitCode::OK,
        None,
    );

    // Expect: GetBytecode(delegate)
    rt.expect_send(
        delegate_id,
        evm::Method::GetBytecode as u64,
        None,
        TokenAmount::from_whole(0),
        None,
        SendFlags::READ_ONLY,
        IpldBlock::serialize_cbor(&Some(bytecode_cid)).unwrap(),
        ExitCode::OK,
        None,
    );

    // Expect value transfer to EOA authority prior to invoking as EOA.
    // The proxy contract sets value=0x42 (66 atto) in the CALL.
    rt.expect_send(
        authority_f4,
        fvm_shared::METHOD_SEND,
        None,
        TokenAmount::from_whole(0x42),
        None,
        SendFlags::empty(),
        None,
        ExitCode::OK,
        None,
    );

    // Gas query for call gas limit.
    rt.expect_gas_available(10_000_000_000u64);

    // Internal self-call with any params; return raw output bytes.
    // Expect delegated execution marker event (topic0 + data=delegate 20b)
    let topic = rt.hash(SupportedHashes::Keccak256, b"EIP7702Delegated(address)");
    rt.expect_emitted_event(ActorEvent::from(vec![
        Entry { flags: Flags::FLAG_INDEXED_ALL, key: "t1".to_owned(), codec: IPLD_RAW, value: topic.clone() },
        Entry { flags: Flags::FLAG_INDEXED_ALL, key: "d".to_owned(), codec: IPLD_RAW, value: delegate_eth.as_ref().to_vec() },
    ]));
    let expected_output = vec![0xca, 0xfe];
    rt.expect_send_any_params(
        rt.receiver,
        evm::Method::InvokeAsEoa as u64,
        TokenAmount::from_whole(0),
        Some(0xffff_ffff),
        SendFlags::empty(),
        Some(IpldBlock { codec: IPLD_RAW, data: expected_output.clone() }),
        ExitCode::OK,
        None,
    );

    // Build call params: [dest(32b)]
    let mut call_params = vec![0u8; 32];
    authority_word.write_as_big_endian(&mut call_params[..]);

    // Invoke and assert output.
    let result = util::invoke_contract(&rt, &call_params);
    assert_eq!(result, expected_output);
}

#[test]
fn call_to_eoa_gas2300_path() {
    let initcode = call::call_proxy_gas2300_contract();
    let rt = util::construct_and_verify(initcode);

    rt.set_network_version(NetworkVersion::V16);

    let authority = EthAddress(hex_literal::hex!("cccccccccccccccccccccccccccccccccccccccc"));
    let authority_word = authority.as_evm_word();

    let delegate_eth = EthAddress(hex_literal::hex!("dddddddddddddddddddddddddddddddddddddddd"));
    let delegate_f4: FilAddress = delegate_eth.into();
    let delegate_id = FilAddress::new_id(0x444u64);
    rt.set_delegated_address(delegate_id.id().unwrap(), delegate_f4);
    rt.actor_code_cids.borrow_mut().insert(delegate_id, *EVM_ACTOR_CODE_ID);

    let bytecode_cid = Cid::try_from("baeaikaia").unwrap();
    rt.store.put_keyed(&bytecode_cid, &[0x00]).unwrap();

    #[derive(fvm_ipld_encoding::serde::Serialize, fvm_ipld_encoding::serde::Deserialize)]
    struct LookupDelegateParams { authority: EthAddress }
    #[derive(fvm_ipld_encoding::serde::Serialize, fvm_ipld_encoding::serde::Deserialize)]
    struct LookupDelegateReturn { delegate: Option<EthAddress> }
    rt.expect_send(
        fil_actors_runtime::DELEGATOR_ACTOR_ADDR,
        frc42_dispatch::method_hash!("LookupDelegate"),
        IpldBlock::serialize_cbor(&LookupDelegateParams { authority }).unwrap(),
        TokenAmount::from_whole(0),
        None,
        SendFlags::READ_ONLY,
        IpldBlock::serialize_cbor(&LookupDelegateReturn { delegate: Some(delegate_eth) }).unwrap(),
        ExitCode::OK,
        None,
    );

    rt.expect_send(
        delegate_id,
        evm::Method::GetBytecode as u64,
        None,
        TokenAmount::from_whole(0),
        None,
        SendFlags::READ_ONLY,
        IpldBlock::serialize_cbor(&Some(bytecode_cid)).unwrap(),
        ExitCode::OK,
        None,
    );

    rt.expect_gas_available(10_000_000_000u64);

    let expected_output = vec![0xaa, 0xbb, 0xcc];
    rt.expect_send_any_params(
        rt.receiver,
        evm::Method::InvokeAsEoa as u64,
        TokenAmount::from_whole(0),
        Some(0xffff_ffff),
        SendFlags::empty(),
        Some(IpldBlock { codec: IPLD_RAW, data: expected_output.clone() }),
        ExitCode::OK,
        None,
    );

    let mut call_params = vec![0u8; 32];
    authority_word.write_as_big_endian(&mut call_params[..]);
    let result = util::invoke_contract(&rt, &call_params);
    assert_eq!(result, expected_output);
}

#[test]
fn call_to_eoa_delegate_reverts_maps_to_zero() {
    let initcode = call::call_proxy_contract();
    let rt = util::construct_and_verify(initcode);
    rt.set_network_version(NetworkVersion::V16);

    let authority = EthAddress(hex_literal::hex!("eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee"));
    let authority_word = authority.as_evm_word();

    let delegate_eth = EthAddress(hex_literal::hex!("ffffffffffffffffffffffffffffffffffffffff"));
    let delegate_f4: FilAddress = delegate_eth.into();
    let delegate_id = FilAddress::new_id(0x555u64);
    rt.set_delegated_address(delegate_id.id().unwrap(), delegate_f4);
    rt.actor_code_cids.borrow_mut().insert(delegate_id, *EVM_ACTOR_CODE_ID);

    let bytecode_cid = Cid::try_from("baeaikaia").unwrap();
    rt.store.put_keyed(&bytecode_cid, &[0x00]).unwrap();

    #[derive(fvm_ipld_encoding::serde::Serialize, fvm_ipld_encoding::serde::Deserialize)]
    struct LookupDelegateParams { authority: EthAddress }
    #[derive(fvm_ipld_encoding::serde::Serialize, fvm_ipld_encoding::serde::Deserialize)]
    struct LookupDelegateReturn { delegate: Option<EthAddress> }
    rt.expect_send(
        fil_actors_runtime::DELEGATOR_ACTOR_ADDR,
        frc42_dispatch::method_hash!("LookupDelegate"),
        IpldBlock::serialize_cbor(&LookupDelegateParams { authority }).unwrap(),
        TokenAmount::from_whole(0),
        None,
        SendFlags::READ_ONLY,
        IpldBlock::serialize_cbor(&LookupDelegateReturn { delegate: Some(delegate_eth) }).unwrap(),
        ExitCode::OK,
        None,
    );

    rt.expect_send(
        delegate_id,
        evm::Method::GetBytecode as u64,
        None,
        TokenAmount::from_whole(0),
        None,
        SendFlags::READ_ONLY,
        IpldBlock::serialize_cbor(&Some(bytecode_cid)).unwrap(),
        ExitCode::OK,
        None,
    );

    rt.expect_gas_available(10_000_000_000u64);

    // InvokeAsEoa returns non-OK; CALL should return 0 and no returndata is copied.
    rt.expect_send_any_params(
        rt.receiver,
        evm::Method::InvokeAsEoa as u64,
        TokenAmount::from_whole(0),
        Some(0xffff_ffff),
        SendFlags::empty(),
        None,
        ExitCode::USR_FORBIDDEN,
        None,
    );

    let mut call_params = vec![0u8; 32];
    authority_word.write_as_big_endian(&mut call_params[..]);
    let result = util::invoke_contract(&rt, &call_params);
    // Expected returndata should reflect CALL success=0 path (we set output to single word 0x42 in base test),
    // but here, since we return 0 and copy no data, the top-level helper returns 32-byte 0 word.
    // We only assert that it is 32-bytes of zeros (starts with 31 zeros).
    assert_eq!(result.len(), 32);
    assert!(result.iter().all(|b| *b == 0));
}
