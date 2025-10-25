use fil_actor_evm as evm;
use fil_actors_evm_shared::address::EthAddress;
use fvm_shared::address::Address as FilAddress;
use fvm_shared::econ::TokenAmount;
use fvm_shared::error::{ErrorNumber, ExitCode};
use fvm_shared::sys::SendFlags;
use fvm_shared::version::NetworkVersion;

mod util;

// Minimal assembler helper for STATICCALL proxy
mod asm_local {
    use etk_asm::ingest::Ingest;
    use evm::interpreter::opcodes;
    use fil_actor_evm as evm;
    pub fn new_contract(
        name: &str,
        init: &str,
        body: &str,
    ) -> Result<Vec<u8>, etk_asm::ingest::Error> {
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
push1 0x00       # out_size
push1 0x00       # out_off
push1 0x20       # in_size = calldatasize - 32
calldatasize
sub
push1 0x00       # in_off
push1 0x00       # load dest
calldataload     # addr
push4 0xffffffff # gas
staticcall
# Return returndata
returndatasize
push1 0x00
push1 0x00
returndatacopy
returndatasize
push1 0x00
return
"#;
    asm_local::new_contract("staticcall-proxy-nv", init, body).unwrap()
}

// Pre-activation: STATICCALL to EOA should not consult Delegator; it should attempt a direct
// InvokeContract to the EOA f4 address and surface NotFound without delegation.
#[test]
fn staticcall_to_eoa_pre_activation_skips_delegation() {
    let initcode = staticcall_proxy_contract();
    let rt = util::construct_and_verify(initcode);

    // Pre-activation
    rt.set_network_version(NetworkVersion::V15);

    let authority = EthAddress(hex_literal::hex!("1212121212121212121212121212121212121212"));
    let authority_f4: FilAddress = authority.into();

    // Expect gas query used for call gas limit computation.
    rt.expect_gas_available(10_000_000_000u64);

    // Expect a direct READ_ONLY send to the EOA f4 address with InvokeContract and NotFound error.
    rt.expect_send(
        authority_f4,
        evm::Method::InvokeContract as u64,
        None,
        TokenAmount::from_whole(0),
        Some(0xffff_ffff),
        SendFlags::READ_ONLY,
        None,
        ExitCode::new(0xffff),
        Some(ErrorNumber::NotFound),
    );

    // Build call params: [dest(32b)] with no additional payload.
    let mut call_params = vec![0u8; 32];
    authority.as_evm_word().write_as_big_endian(&mut call_params[..]);

    // Invoke the contract; we only care that no unexpected Delegator send occurred.
    let _ = util::invoke_contract(&rt, &call_params);
    rt.verify();
}
