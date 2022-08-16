use super::{Message, TransactionAction, H160};

// TODO probably have a different type of input here (probably a deserialized message)
pub fn is_precompiled(msg: &TransactionAction) -> bool {
    if let TransactionAction::Call(addr) = msg {
        !addr.is_zero() && addr <= &MAX_PRECOMPILE
    } else {
        false
    }
}

// TODO cleanup
#[derive(Debug)]
pub struct PrecompileOutput {
    pub cost: u64,
    pub output: Vec<u8>,
}

pub type PrecompileFn = fn(&[u8], u64) -> Result<PrecompileOutput, ()>; // TODO useful error

fn nop(inp: &[u8], c: u64) -> Result<PrecompileOutput, ()> {
    todo!()
}
// TODO pull in new ref-fvm with new hash fn
fn hash_syscall(mh_code: u64, input: &[u8]) -> Result<Vec<u8>, ()> {
    todo!()
}

fn sha256(inp: &[u8], c: u64) -> Result<PrecompileOutput, ()> {
    let dynamic_gas = 0; // TODO
    Ok(PrecompileOutput { cost: 60 + dynamic_gas, output: hash_syscall(0, inp)? })
}

/// List of precompile smart contracts, index + 1 is the address (another option is to make an enum)
const PRECOMPILES: [PrecompileFn; 9] = [
    nop,    // ecrecover 0x01
    sha256, // SHA2_256 0x02
    nop,    // ripemd160 0x03
    nop,    // identity 0x04
    nop,    // modexp 0x05
    nop,    // ecAdd 0x06
    nop,    // ecMul 0x07
    nop,    // ecPairing 0x08
    nop,    // blake2f 0x09
];

const MAX_PRECOMPILE: H160 = {
    let mut bytes = [0u8; 20];
    bytes[0] = PRECOMPILES.len() as u8;
    H160(bytes)
};

pub fn call_precompile(msg: &mut Message) {
    let precompile_num = msg.recipient.0[0] as usize;

    let res = PRECOMPILES[precompile_num](&msg.input_data, 0);

    todo!()
}
