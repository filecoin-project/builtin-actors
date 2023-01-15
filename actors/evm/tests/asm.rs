use etk_asm::ingest::Ingest;
use evm::interpreter::execution::opcodes;
use fil_actor_evm as evm;

const PRELUDE: &str = r#"
%macro dispatch_begin()
  push1 0x00
  calldataload
  push1 0xe0   # 28 byte shift == 224 bits
  shr
%end

%macro dispatch(method, lbl)
  dup1
  %push($method)
  eq
  %push($lbl)
  jumpi
%end

%macro dispatch_end()
  push1 0x00
  dup1
  revert
%end

%macro return_stack_word()
    # store at 0x00
    push1 0x00
    mstore
    push1 0x20 # always return a full word
    push1 0x00
    return
%end
"#;

#[allow(dead_code)]
/// Creates a new EVM contract constructon bytecode (initcode), suitable for initializing the EVM actor.
/// Arguments:
/// - name is the name of the contract, for debug purposes.
/// - init is the initializer code, which will run first at contract construction.
/// - body is the actual contract code.
pub fn new_contract(name: &str, init: &str, body: &str) -> Result<Vec<u8>, etk_asm::ingest::Error> {
    // the contract code
    let mut body_code = Vec::new();
    let mut ingest_body = Ingest::new(&mut body_code);
    let body_with_prelude = PRELUDE.to_owned() + &body;
    ingest_body.ingest(name, body_with_prelude.as_str())?;
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
    // the actual contract code
    let mut contract_code = Vec::new();
    contract_code.append(&mut init_code);
    contract_code.append(&mut constructor_code);
    contract_code.append(&mut body_code);
    Ok(contract_code)
}
