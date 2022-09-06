use etk_asm::ingest::Ingest;
use evm::interpreter::opcode::OpCode::*;
use fil_actor_evm as evm;

#[allow(dead_code)]
pub fn new_contract(name: &str, init: &str, body: &str) -> Result<Vec<u8>, etk_asm::ingest::Error> {
    // the contract code
    let mut body_code = Vec::new();
    let mut ingest_body = Ingest::new(&mut body_code);
    ingest_body.ingest(name, body)?;
    // the initialization code
    let mut init_code = Vec::new();
    let mut ingest_init = Ingest::new(&mut init_code);
    ingest_init.ingest(name, init)?;
    // synthesize contract constructor
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
        + 1 // RETURN
        ;
    let mut constructor_code = Vec::<u8>::from([
        PUSH4 as u8,
        ((body_code_len >> 24) & 0xff) as u8,
        ((body_code_len >> 16) & 0xff) as u8,
        ((body_code_len >> 8) & 0xff) as u8,
        (body_code_len & 0xff) as u8,
        DUP1 as u8,
        PUSH4 as u8,
        ((body_code_offset >> 24) & 0xff) as u8,
        ((body_code_offset >> 16) & 0xff) as u8,
        ((body_code_offset >> 8) & 0xff) as u8,
        (body_code_offset & 0xff) as u8,
        PUSH1 as u8,
        0x00,
        CODECOPY as u8,
        PUSH1 as u8,
        0x00,
        RETURN as u8,
    ]);
    // the actual contract code
    let mut contract_code = Vec::new();
    contract_code.append(&mut init_code);
    contract_code.append(&mut constructor_code);
    contract_code.append(&mut body_code);
    Ok(contract_code)
}
