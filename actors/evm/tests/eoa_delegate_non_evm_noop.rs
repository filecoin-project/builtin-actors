use fil_actor_evm as evm;
use fil_actors_evm_shared::address::EthAddress;
use fil_actors_runtime::runtime::Primitives;
use fil_actors_runtime::test_utils::{MockRuntime, ACCOUNT_ACTOR_CODE_ID, ETHACCOUNT_ACTOR_CODE_ID};
use fvm_ipld_encoding::ipld_block::IpldBlock;
use fvm_shared::address::Address as FilAddress;
use fvm_shared::econ::TokenAmount;
use fvm_shared::error::ExitCode;
use fvm_shared::sys::SendFlags;
use fvm_shared::version::NetworkVersion;

mod util;

// Local helper to assemble a simple CALL proxy contract.
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

fn call_proxy_contract() -> Vec<u8> {
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
    asm_local::new_contract("call-proxy", init, body).unwrap()
}

#[test]
fn delegate_to_non_evm_treated_as_noop_success() {
    // Construct proxy contract and enable EIP‑7702.
    let initcode = call_proxy_contract();
    let rt = util::construct_and_verify(initcode);
    rt.set_network_version(NetworkVersion::V16);

    // EOA authority destination.
    let authority = EthAddress(hex_literal::hex!("1010101010101010101010101010101010101010"));
    let authority_word = authority.as_evm_word();

    // Delegate is a non‑EVM actor (use account or ethaccount actor code id).
    let delegate_eth = EthAddress(hex_literal::hex!("2020202020202020202020202020202020202020"));
    let delegate_f4: FilAddress = delegate_eth.into();
    let delegate_id = FilAddress::new_id(0xabc);
    rt.set_delegated_address(delegate_id.id().unwrap(), delegate_f4);
    rt.actor_code_cids.borrow_mut().insert(delegate_id, *ACCOUNT_ACTOR_CODE_ID);

    // Delegator returns mapping for authority -> delegate.
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

    // Build call params and invoke contract. Since delegate is not EVM, the call path should
    // treat as success with no‑op; thus no returndata is copied (empty vector).
    let mut call_params = vec![0u8; 32];
    authority_word.write_as_big_endian(&mut call_params[..]);
    let result = util::invoke_contract(&rt, &call_params);
    assert!(result.is_empty());
}
