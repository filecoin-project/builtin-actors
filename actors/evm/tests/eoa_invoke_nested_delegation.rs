use cid::Cid;
use fil_actor_evm as evm;
use fil_actors_evm_shared::address::EthAddress;
use fil_actors_runtime::runtime::Primitives;
use fil_actors_runtime::test_utils::EVM_ACTOR_CODE_ID;
use fvm_ipld_blockstore::Blockstore;
use fvm_ipld_encoding::ipld_block::IpldBlock;
use fvm_shared::address::Address as FilAddress;
use fvm_shared::econ::TokenAmount;
use fvm_shared::error::ExitCode;
use fvm_shared::event::{ActorEvent, Entry, Flags};
use fvm_shared::sys::SendFlags;
use fvm_shared::version::NetworkVersion;
use fvm_shared::IPLD_RAW;

mod util;

mod asm_local {
    use etk_asm::ingest::Ingest;
    use fil_actor_evm as evm;
    use evm::interpreter::opcodes;
    pub fn new_contract(name: &str, init: &str, body: &str) -> Result<Vec<u8>, etk_asm::ingest::Error> {
        let mut body_code = Vec::new();
        let mut ingest_body = Ingest::new(&mut body_code);
        ingest_body.ingest(name, body)?;
        let mut init_code = Vec::new();
        let mut ingest_init = Ingest::new(&mut init_code);
        ingest_init.ingest(name, init)?;
        let body_code_len = body_code.len();
        let body_code_offset = init_code.len() + 1 + 4 + 1 + 1 + 4 + 1 + 1 + 1 + 1 + 1 + 1;
        let mut constructor_code = vec![
            opcodes::PUSH4,
            ((body_code_len >> 24) & 0xff) as u8,
            ((body_code_len >> 16) & 0xff) as u8,
            ((body_code_len >> 8) & 0xff) as u8,
            (body_code_len & 0xff) as u8,
            opcodes::DUP1,
            opcodes::PUSH4,
            ((body_code_offset >> 24) & 0xff) as u8,
            ((body_code_offset >> 16) & 0xff) as u8,
            ((body_code_offset >> 8) & 0xff) as u8,
            (body_code_offset & 0xff) as u8,
            opcodes::PUSH1,
            0x00,
            opcodes::CODECOPY,
            opcodes::PUSH1,
            0x00,
            opcodes::RETURN,
        ];
        let mut contract_code = Vec::new();
        contract_code.append(&mut init_code);
        contract_code.append(&mut constructor_code);
        contract_code.append(&mut body_code);
        Ok(contract_code)
    }
}

#[test]
fn invoke_as_eoa_nested_delegation_behavior() {
    // Construct an EVM actor (receiver) as usual.
    let rt = util::construct_and_verify(vec![0x00]); // minimal contract bytecode (STOP)
    rt.set_network_version(NetworkVersion::V16);

    // Authority A and nested authority B
    let authority_a = EthAddress(hex_literal::hex!("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"));
    let authority_b = EthAddress(hex_literal::hex!("bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"));

    // Delegate1 (for A): its bytecode will CALL using the first 32 bytes of input as the destination.
    let delegate1_id = FilAddress::new_id(0x301);
    let delegate1_eth = EthAddress(hex_literal::hex!("1111111111111111111111111111111111111111"));
    let delegate1_f4: FilAddress = delegate1_eth.into();
    rt.set_delegated_address(delegate1_id.id().unwrap(), delegate1_f4);
    rt.actor_code_cids.borrow_mut().insert(delegate1_id, *EVM_ACTOR_CODE_ID);
    let delegate1_code = {
        // Use the same call-proxy body as other tests.
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
        asm_local::new_contract("delegate1-call-proxy", init, body).unwrap()
    };
    let delegate1_cid = Cid::try_from("baeaikaia").unwrap();
    rt.store.put_keyed(&delegate1_cid, &delegate1_code).unwrap();

    // Delegate2 (for B): STOP
    let delegate2_id = FilAddress::new_id(0x302);
    let delegate2_eth = EthAddress(hex_literal::hex!("2222222222222222222222222222222222222222"));
    let delegate2_f4: FilAddress = delegate2_eth.into();
    rt.set_delegated_address(delegate2_id.id().unwrap(), delegate2_f4);
    rt.actor_code_cids.borrow_mut().insert(delegate2_id, *EVM_ACTOR_CODE_ID);
    let delegate2_cid = Cid::try_from("baeaikaia").unwrap();
    rt.store.put_keyed(&delegate2_cid, &[0x00]).unwrap();

    // Expectations for outer InvokeAsEoa (receiver bound to A)
    // 1) Mount existing storage root for A
    #[derive(fvm_ipld_encoding::serde::Serialize, fvm_ipld_encoding::serde::Deserialize)]
    struct GetStorageRootParams { authority: EthAddress }
    #[derive(fvm_ipld_encoding::serde::Serialize, fvm_ipld_encoding::serde::Deserialize)]
    struct GetStorageRootReturn { root: Option<Cid> }
    rt.expect_send(
        fil_actors_runtime::DELEGATOR_ACTOR_ADDR,
        frc42_dispatch::method_hash!("GetStorageRoot"),
        IpldBlock::serialize_cbor(&GetStorageRootParams { authority: authority_a }).unwrap(),
        TokenAmount::from_whole(0),
        None,
        SendFlags::READ_ONLY,
        IpldBlock::serialize_cbor(&GetStorageRootReturn { root: None }).unwrap(),
        ExitCode::OK,
        None,
    );

    // Nested path during execution: delegate1 issues CALL to B.
    // The EVM will consult Delegator and resolve delegate2, emit event, and attempt a nested InvokeAsEoa.
    #[derive(fvm_ipld_encoding::serde::Serialize, fvm_ipld_encoding::serde::Deserialize)]
    struct LookupDelegateParams { authority: EthAddress }
    #[derive(fvm_ipld_encoding::serde::Serialize, fvm_ipld_encoding::serde::Deserialize)]
    struct LookupDelegateReturn { delegate: Option<EthAddress> }
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
        IpldBlock::serialize_cbor(&Some(delegate2_cid)).unwrap(),
        ExitCode::OK,
        None,
    );

    rt.expect_gas_available(10_000_000_000u64);
    let topic = rt.hash(fvm_shared::crypto::hash::SupportedHashes::Keccak256, b"EIP7702Delegated(address)");
    rt.expect_emitted_event(ActorEvent::from(vec![
        Entry { flags: Flags::FLAG_INDEXED_ALL, key: "t1".to_owned(), codec: IPLD_RAW, value: topic.clone() },
        Entry { flags: Flags::FLAG_INDEXED_ALL, key: "d".to_owned(), codec: IPLD_RAW, value: delegate2_eth.as_ref().to_vec() },
    ]));
    // Nested self-call: we don't execute it; just assert it is attempted.
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

    // After execution, persist A's storage root (accept any params).
    rt.expect_send_any_params(
        fil_actors_runtime::DELEGATOR_ACTOR_ADDR,
        frc42_dispatch::method_hash!("PutStorageRoot"),
        TokenAmount::from_whole(0),
        None,
        SendFlags::empty(),
        None,
        ExitCode::OK,
        None,
    );

    // Set caller validation for InvokeAsEoa and call it directly.
    rt.expect_validate_caller_addr(vec![rt.receiver]);
    rt.set_caller(*EVM_ACTOR_CODE_ID, rt.receiver);

    // InvokeAsEoa with delegate1 code, receiver=A, and input containing B at offset 0.
    let params = evm::EoaInvokeParams {
        code: delegate1_cid,
        input: authority_b.as_ref().to_vec(),
        caller: EthAddress::from_id(0x999),
        receiver: authority_a,
        value: TokenAmount::from_whole(0),
    };
    let _ = rt.call::<evm::EvmContractActor>(
        evm::Method::InvokeAsEoa as u64,
        IpldBlock::serialize_dag_cbor(&params).unwrap(),
    );
    rt.verify();
}
