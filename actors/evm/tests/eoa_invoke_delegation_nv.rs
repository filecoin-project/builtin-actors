use fil_actor_evm as evm;
use fil_actors_evm_shared::address::EthAddress;
use fvm_shared::address::Address as FilAddress;
use fvm_shared::econ::TokenAmount;
use fvm_shared::error::{ErrorNumber, ExitCode};
use fvm_shared::sys::SendFlags;

mod util;

// Minimal assembler helper (copied from tests/asm.rs)
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
        let body_code_offset = init_code.len()
            + 1 // PUSH4
            + 4 // 4-bytes for code length
            + 1 // DUP1
            + 1 // PUSH4
            + 4 // 4 bytes for the code offset itself
            + 1 // PUSH1
            + 1 // 0x00 -- destination memory offset
            + 1 // CODECOPY
            + 1 // PUSH1
            + 1 // 0x00 -- source memory offset
            + 1; // RETURN
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
# this contract takes an address and the call payload and proxies a call to that address
# get call payload size
push1 0x20
calldatasize
sub
# store payload to mem 0x00
push1 0x20
push1 0x00
calldatacopy

# prepare the proxy call
# output offset and size -- 0 in this case, we use returndata
push2 0x00
push1 0x00
# input offset and size
push1 0x20
calldatasize
sub
push1 0x00
# value
push1 0x00
# dest address
push1 0x00
calldataload
# gas
push4 0xffffffff
# do the call
call

# return result through
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

// Pre-activation: CALL to EOA should not consult Delegator;
// it should attempt a direct InvokeContract on the EOA f4 address and surface the syscall error.
#[test]
fn call_to_eoa_pre_activation_skips_delegator() {
    // Construct a proxy contract that CALLs a destination and returns returndata.
    let initcode = call_proxy_contract();
    let rt = util::construct_and_verify(initcode);

    // Destination is an EOA (no actor code registered, NotFound).
    let authority = EthAddress(hex_literal::hex!("1111222233334444555566667777888899990000"));
    let authority_f4: FilAddress = authority.into();

    // Build call params: [dest(32b)] with no additional payload.
    let mut call_params = vec![0u8; 32];
    authority.as_evm_word().write_as_big_endian(&mut call_params[..]);

    // Expect gas query when computing call gas limit.
    rt.expect_gas_available(10_000_000_000u64);

    // Expect a direct send to the EOA f4 address with InvokeContract and NotFound error.
    rt.expect_send(
        authority_f4,
        evm::Method::InvokeContract as u64,
        None,
        TokenAmount::from_whole(0),
        Some(0xffff_ffff),
        SendFlags::empty(),
        None,
        ExitCode::new(0xffff),
        Some(ErrorNumber::NotFound),
    );

    // Invoke the contract; we only care that no unexpected Delegator send occurred.
    let _ = util::invoke_contract(&rt, &call_params);
    rt.verify();
}
