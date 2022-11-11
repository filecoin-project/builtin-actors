use etk_asm::ingest::Ingest;
use evm::interpreter::opcode::OpCode::*;
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
    let body = with_fevm_extensions(body);
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
    ];
    // the actual contract code
    let mut contract_code = Vec::new();
    contract_code.append(&mut init_code);
    contract_code.append(&mut constructor_code);
    contract_code.append(&mut body_code);
    Ok(contract_code)
}

// this is a hack to support mnemonics for the FEVM extension opcodes
// it is really ugly, but the etk assmebler doesn't currently support any way to
// directly embed (otherwise invalid) asm instructions in the stream... sigh.
// Ideally we would just do them as macros like
// %macro methodnum()
//   0xb1
// %end
// Note that to add insult to injury, macros cannot %include_hex... double sigh.
// So f*ck it, we'll just hack this until there is support.
// See also https://github.com/quilt/etk/issues/110
fn with_fevm_extensions(body: &str) -> String {
    body.to_owned().replace("@callactor", "%include_hex(\"tests/opcodes/callactor.hex\")")
}
