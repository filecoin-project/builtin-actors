use cid::Cid;
use fil_actor_evm as evm;
use fil_actors_evm_shared::address::EthAddress;
use fil_actors_runtime::test_utils::EVM_ACTOR_CODE_ID;
use fvm_ipld_blockstore::Blockstore;
use fvm_ipld_encoding::ipld_block::IpldBlock;
use fvm_shared::address::Address as FilAddress;
use fvm_shared::econ::TokenAmount;
use fvm_shared::error::ExitCode;
use fvm_shared::sys::SendFlags;
use fvm_shared::event::{ActorEvent, Entry, Flags};
use fil_actors_runtime::runtime::Primitives;
use fvm_shared::crypto::hash::SupportedHashes;
use fvm_shared::version::NetworkVersion;

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

fn staticcall_proxy_contract() -> Vec<u8> {
    let init = "";
    let body = r#"
# Prepare STATICCALL(gas, addr, in_off, in_size, out_off, out_size)
# We don't provide an output buffer; we'll copy returndata after.
push1 0x00       # out_size
push1 0x00       # out_off
push1 0x20       # compute in_size = calldatasize - 32
calldatasize
sub
push1 0x00       # in_off
push1 0x00       # calldataload offset for addr
calldataload     # addr
push4 0xffffffff # gas
staticcall

# Copy returndata to memory and return it
returndatasize
push1 0x00
push1 0x00
returndatacopy
returndatasize
push1 0x00
return
"#;
    asm_local::new_contract("staticcall-proxy", init, body).unwrap()
}

#[test]
fn staticcall_to_eoa_delegates_readonly() {
    let initcode = staticcall_proxy_contract();
    let rt = util::construct_and_verify(initcode);
    rt.set_network_version(NetworkVersion::V16);

    let authority = EthAddress(hex_literal::hex!("1212121212121212121212121212121212121212"));
    let authority_word = authority.as_evm_word();

    let delegate_eth = EthAddress(hex_literal::hex!("3434343434343434343434343434343434343434"));
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

    // No value transfer expected in staticcall path.
    rt.expect_gas_available(10_000_000_000u64);

    // Expect delegated execution marker event (topic0 + data=delegate 20b)
    let topic = rt.hash(SupportedHashes::Keccak256, b"EIP7702Delegated(address)");
    rt.expect_emitted_event(ActorEvent::from(vec![
        Entry { flags: Flags::FLAG_INDEXED_ALL, key: "t1".to_owned(), codec: fvm_shared::IPLD_RAW, value: topic.clone() },
        Entry { flags: Flags::FLAG_INDEXED_ALL, key: "d".to_owned(), codec: fvm_shared::IPLD_RAW, value: delegate_eth.as_ref().to_vec() },
    ]));

    let expected_output = vec![0xaa, 0xbb];
    rt.expect_send_any_params(
        rt.receiver,
        evm::Method::InvokeAsEoa as u64,
        TokenAmount::from_whole(0),
        Some(0xffff_ffff),
        SendFlags::empty(),
        Some(IpldBlock { codec: fvm_shared::IPLD_RAW, data: expected_output.clone() }),
        ExitCode::OK,
        None,
    );

    let mut call_params = vec![0u8; 32];
    authority_word.write_as_big_endian(&mut call_params[..]);
    let result = util::invoke_contract(&rt, &call_params);
    assert_eq!(result, expected_output);
}
