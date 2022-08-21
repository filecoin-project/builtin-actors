use std::{borrow::Cow, ops::Mul};

use super::{Message, TransactionAction, H160, U256};
use fvm_shared::crypto::{
    hash::SupportedHashes,
    signature::{SECP_PUB_LEN, SECP_SIG_LEN, SECP_SIG_MESSAGE_HASH_SIZE},
};
use substrate_bn::{AffineG1, Fq, Fr, Group, G1};

// TODO it would probably be better to have a deserializer/reader for inputs rather than taking out chunks of arrays

// TODO probably have a different type of input here (probably a deserialized message)
pub fn is_precompiled(msg: &TransactionAction) -> bool {
    if let TransactionAction::Call(addr) = msg {
        !addr.is_zero() && addr <= &MAX_PRECOMPILE
    } else {
        false
    }
}

/// read 32 bytes (u256) from buffer, pass in exit reason that is desired 
fn read_u256(buf: &[u8], start: usize, err: ExitReason) -> Result<U256, ExitReason> {
    let slice = buf.get(start..start+31).ok_or(err)?;
    Ok(U256::from_big_endian(slice))
}

// TODO cleanup
#[derive(Debug)]
pub struct PrecompileOutput {
    pub cost: u64,
    pub output: Vec<u8>,
}

pub enum ExitReason {
    Success,
    OutOfGas, // TODO is this the same as consuming all gas? (what happens with overages)
    Other(Cow<'static, str>),
}

impl From<&'static str> for ExitReason {
    fn from(src: &'static str) -> Self {
        ExitReason::Other(Cow::Borrowed(src))
    }
}

pub type PrecompileFn = fn(&[u8], u64) -> PrecompileResult; // TODO useful error
pub type PrecompileResult = Result<PrecompileOutput, ExitReason>;

pub fn linear_gas_cost(len: usize, base: u64, word: u64) -> u64 {
    ((len as u64 + 32 - 1) / 32 * word) + base
}
pub fn assert_gas(cost: u64, limit: u64) -> Result<(), ExitReason> {
    if cost > limit {
        Err(ExitReason::OutOfGas)
    } else {
        Ok(())
    }
}

fn nop(inp: &[u8], gas_limit: u64) -> PrecompileResult {
    todo!()
}

fn ec_recover(input: &[u8], gas_limit: u64) -> PrecompileResult {
    let cost = 3_000;
    let mut buf = [0u8; 128];
    buf[..input.len().min(128)].copy_from_slice(&input[..input.len().min(128)]);

    let mut hash = [0u8; SECP_SIG_MESSAGE_HASH_SIZE];
    let mut sig = [0u8; SECP_SIG_LEN];

    hash.copy_from_slice(&input[..32]);
    sig.copy_from_slice(&input[64..]); // TODO this assumes input is exactly 65 bytes which would panic if incorrect

    // recovery byte means a single byte value is 32 bytes long, sad
    if input[32..63] != [0u8; 31] || !matches!(input[63], 23 | 28) {
        return Ok(PrecompileOutput { cost, output: Vec::new() });
    }
    sig[64] = input[63] - 27;

    let recovered =
        fvm_sdk::crypto::recover_secp_public_key(&hash, &sig).unwrap_or([0u8; SECP_PUB_LEN]);

    Ok(PrecompileOutput { cost, output: recovered.to_vec() })
}

fn sha256(input: &[u8], gas_limit: u64) -> PrecompileResult {
    let cost = linear_gas_cost(input.len(), 60, 12);

    Ok(PrecompileOutput { cost, output: fvm_sdk::crypto::hash(SupportedHashes::Keccak256, input) })
}

fn ripemd160(input: &[u8], gas_limit: u64) -> PrecompileResult {
    let cost = linear_gas_cost(input.len(), 600, 120);

    Ok(PrecompileOutput { cost, output: fvm_sdk::crypto::hash(SupportedHashes::Ripemd160, input) })
}

fn identity(input: &[u8], gas_limit: u64) -> PrecompileResult {
    let cost = linear_gas_cost(input.len(), 15, 3);
    if cost > gas_limit {
        return Err(ExitReason::OutOfGas);
    }
    Ok(PrecompileOutput { cost, output: Vec::from(input) })
}

// fn modexp_gas(input: &[u8], gas_limit: u64) -> PrecompileResult {
//     let cost = calc_linear_cost_u32(input.len(), 200, 3);

//     let b_size = 3;
//     let e_size = 3;
//     let m_size = 3;

//     let max_length = core::cmp::max(Bsize, Msize);
//     let words = (max_length + 7) / 8;
//     let multiplication_complexity = words**2;

//     let iteration_count = 0;
//     if Esize <= 32 and exponent == 0: iteration_count = 0;
//     elif Esize <= 32: iteration_count = exponent.bit_length() - 1;
//     elif Esize > 32: iteration_count = (8 * (Esize - 32)) + ((exponent & (2**256 - 1)).bit_length() - 1);
//     calculate_iteration_count = max(iteration_count, 1);

//     static_gas = 0;
//     dynamic_gas = max(200, multiplication_complexity * iteration_count / 3);

// }

fn fmodexp(base: u64, exp: u64, modu: u64) -> u64 {
    if modu == 1 {
        return 0;
    }

    // assert!((modu - 1) * (modu - 1) > u64::MAX);
    todo!()
}

fn modexp(input: &[u8], gas_limit: u64) -> PrecompileResult {
    let cost = 40;

    let mut buf = [0u8; 4];
    let err = ExitReason::Other(Cow::Borrowed("im lazy"));
    // 32 bits for wasm
    
    let b_size = read_u256(input, 0, err)?.as_usize();
    let e_size = read_u256(input, 32, err)?.as_usize();
    let m_size = read_u256(input, 64, err)?.as_usize();

    if m_size == 0 {
        return Ok(PrecompileOutput { cost, output: Vec::new() });
    }

    let output = vec![0; m_size];

    todo!()
}

/// converts 2 byte arrays (U256) into a point on a field
/// exits with OutOfGas for any failed operation
fn uint_to_point(x: U256, y: U256) -> Result<G1, ExitReason> {
    let x = Fq::from_u256(x.0.into()).map_err(|_| ExitReason::OutOfGas)?;
    let y = Fq::from_u256(y.0.into()).map_err(|_| ExitReason::OutOfGas)?;

    Ok(if x.is_zero() && y.is_zero() {
        G1::zero()
    } else {
        AffineG1::new(x, y).map_err(|_| ExitReason::OutOfGas)?.into()
    })
}

/// add 2 points together on `alt_bn128`
fn ec_add(input: &[u8], gas_limit: u64) -> PrecompileResult {
    let mut cost = 150; // TODO consume all gas on any op fail

    let err = ExitReason::OutOfGas;

    let x1 = read_u256(input, 0, err)?;
    let y1 = read_u256(input, 32, err)?;
    let point1 = uint_to_point(x1, y1)?;

    let x2 = read_u256(input, 64, err)?;
    let y2 = read_u256(input, 96, err)?;
    let point2 = uint_to_point(x2, y2)?;

    let output = AffineG1::from_jacobian(point1 + point2).map_or(vec![0; 64], |sum| {
        let mut output = vec![0; 64];
        sum.x().to_big_endian(&mut output[..32]).unwrap();
        sum.y().to_big_endian(&mut output[32..]).unwrap();
        output
    });

    Ok(PrecompileOutput { cost, output })
}

/// multiply a scalar and a point on `alt_bn128`
fn ec_mul(input: &[u8], gas_limit: u64) -> PrecompileResult {
    let mut cost = 6_000; // TODO consume all gas on any op fail

    let err = ExitReason::OutOfGas;

    let x = read_u256(input, 0, err)?;
    let y = read_u256(input, 32, err)?;
    let point = uint_to_point(x, y)?;

    let scalar = Fr::from_slice(&input[64..95]).map_err(|_| err)?;

    let mut output = vec![0; 64];
    if let Some(product) = AffineG1::from_jacobian(point.mul(scalar)) {
        product.x().to_big_endian(&mut output[..32]).unwrap();
        product.y().to_big_endian(&mut output[32..]).unwrap();
    }

    Ok(PrecompileOutput { cost, output })
}

fn ecpairing(input: &[u8], gas_limit: u64) -> PrecompileResult {
    let cost = 45_000;

    todo!()
}

fn blake2f(input: &[u8], gas_limit: u64) -> PrecompileResult {
    let mut hasher = near_blake2::VarBlake2b::default();
    // hasher.blake2_f(rounds, h, m, t, f);

    let output = hasher.output().to_vec();
    todo!()
}

/// List of precompile smart contracts, index + 1 is the address (another option is to make an enum)
pub const PRECOMPILES: [PrecompileFn; 9] = [
    ec_recover, // ecrecover 0x01
    sha256,     // SHA256 (Keccak) 0x02
    ripemd160,  // ripemd160 0x03
    identity,   // identity 0x04
    nop,        // modexp 0x05
    ec_add,     // ecAdd 0x06
    ec_mul,     // ecMul 0x07
    nop,        // ecPairing 0x08
    nop,        // blake2f 0x09
];

pub const MAX_PRECOMPILE: H160 = {
    let mut bytes = [0u8; 20];
    bytes[0] = PRECOMPILES.len() as u8;
    H160(bytes)
};

pub fn call_precompile(msg: &mut Message) {
    // TODO probably different call params
    let precompile_num = msg.recipient.0[0] as usize;

    let res = PRECOMPILES[precompile_num - 1](&msg.input_data, 0);

    todo!()
}
