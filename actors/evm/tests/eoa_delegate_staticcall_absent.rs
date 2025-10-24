use fil_actor_evm as evm;
use fil_actors_evm_shared::address::EthAddress;
use fvm_ipld_encoding::ipld_block::IpldBlock;
use fvm_shared::address::Address as FilAddress;
use fvm_shared::econ::TokenAmount;
use fvm_shared::error::ExitCode;
use fvm_shared::sys::SendFlags;
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
push1 0x00       # out_size
push1 0x00       # out_off
push1 0x20       # in_size
calldatasize
sub
push1 0x00       # in_off
push1 0x00       # load dest
calldataload
push4 0xffffffff # gas
staticcall
returndatasize
push1 0x00
push1 0x00
returndatacopy
returndatasize
push1 0x00
return
"#;
    asm_local::new_contract("staticcall-proxy-absent", init, body).unwrap()
}

#[test]
fn staticcall_to_eoa_no_mapping_direct_call() {
    let initcode = staticcall_proxy_contract();
    let rt = util::construct_and_verify(initcode);
    rt.set_network_version(NetworkVersion::V16);

    let authority = EthAddress(hex_literal::hex!("abababababababababababababababababababab"));
    let authority_word = authority.as_evm_word();
    let authority_f4: FilAddress = authority.into();

    // Delegator LookupDelegate(authority) -> None
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
        IpldBlock::serialize_cbor(&LookupDelegateReturn { delegate: None }).unwrap(),
        ExitCode::OK,
        None,
    );

    // Gas query
    rt.expect_gas_available(10_000_000_000u64);

    // Expect: direct READ_ONLY InvokeContract to EOA (no event expected).
    let expected_output = vec![0xaa, 0xbb, 0xcc];
    rt.expect_send(
        authority_f4,
        evm::Method::InvokeContract as u64,
        None,
        TokenAmount::from_whole(0),
        Some(0xffff_ffff),
        SendFlags::READ_ONLY,
        Some(IpldBlock { codec: fvm_shared::IPLD_RAW, data: expected_output.clone() }),
        ExitCode::OK,
        None,
    );

    let mut call_params = vec![0u8; 32];
    authority_word.write_as_big_endian(&mut call_params[..]);
    let result = util::invoke_contract(&rt, &call_params);
    assert_eq!(result, expected_output);
}
