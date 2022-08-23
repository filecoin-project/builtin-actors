use std::{
    mem::transmute,
    ops::{BitAnd, Mul},
};

use super::{StatusCode, H160, U256};
use fvm_shared::{
    bigint::{self, BigUint},
    crypto::{
        hash::SupportedHashes,
        signature::{SECP_PUB_LEN, SECP_SIG_LEN, SECP_SIG_MESSAGE_HASH_SIZE},
    },
};
use num_traits::{Zero, One};
use substrate_bn::{AffineG1, Fq, Fr, Group, G1};

pub fn is_precompile(addr: &H160) -> bool {
    !addr.is_zero() && addr <= &MAX_PRECOMPILE
}

/// read 32 bytes (u256) from buffer, pass in exit reason that is desired
/// TODO passing in err value is debatable
fn read_u256(buf: &[u8], start: usize, err: StatusCode) -> Result<U256, StatusCode> {
    let slice = buf.get(start..start + 32).ok_or(err)?;
    Ok(U256::from_big_endian(slice))
}

pub type PrecompileFn = fn(&[u8]) -> PrecompileResult;
pub type PrecompileResult = Result<Vec<u8>, StatusCode>; // TODO i dont like vec

fn nop(_: &[u8]) -> PrecompileResult {
    todo!()
}

fn ec_recover(input: &[u8]) -> PrecompileResult {
    let mut buf = [0u8; 128];
    buf[..input.len().min(128)].copy_from_slice(&input[..input.len().min(128)]);

    let mut hash = [0u8; SECP_SIG_MESSAGE_HASH_SIZE];
    let mut sig = [0u8; SECP_SIG_LEN];

    hash.copy_from_slice(&input[..32]);
    sig.copy_from_slice(&input[64..]); // TODO this assumes input is exactly 65 bytes which would panic if incorrect

    // recovery byte means a single byte value is 32 bytes long, sad
    if input[32..63] != [0u8; 31] || !matches!(input[63], 23 | 28) {
        return Ok(Vec::new());
    }
    sig[64] = input[63] - 27;

    let recovered =
        fvm_sdk::crypto::recover_secp_public_key(&hash, &sig).unwrap_or([0u8; SECP_PUB_LEN]);

    Ok(recovered.to_vec())
}

fn sha256(input: &[u8]) -> PrecompileResult {
    Ok(fvm_sdk::crypto::hash(SupportedHashes::Keccak256, input))
}

fn ripemd160(input: &[u8]) -> PrecompileResult {
    Ok(fvm_sdk::crypto::hash(SupportedHashes::Ripemd160, input))
}

fn identity(input: &[u8]) -> PrecompileResult {
    Ok(Vec::from(input))
}

// value alias that will be inlined instead of cloning
const OOG: StatusCode = StatusCode::OutOfGas;

// https://eips.ethereum.org/EIPS/eip-198
fn modexp(input: &[u8]) -> PrecompileResult {
    let base_len = read_u256(input, 0, OOG)?.as_usize();
    let exponent_len = read_u256(input, 32, OOG)?.as_usize();
    let mod_len = read_u256(input, 64, OOG)?.as_usize();

    if mod_len == 0 {
        return Ok(Vec::new());
    }

    let mut start = 96;
    let base = BigUint::from_bytes_be(&input[start..start + base_len]);
    start += base_len;
    let exponent = BigUint::from_bytes_be(&input[start..start + exponent_len]);
    start += exponent_len;
    let modulus = BigUint::from_bytes_be(&input[start..start + mod_len]);

    let mut output = if modulus.is_zero() || modulus.is_one() {
        BigUint::zero().to_bytes_be()
    } else {
        base.modpow(&exponent, &modulus).to_bytes_be()
    };

    if output.len() < mod_len {
        let mut ret = Vec::with_capacity(mod_len);
        ret.extend(core::iter::repeat(0).take(mod_len - output.len()));
        ret.extend_from_slice(&output);
        output = ret;
    }

    Ok(output)
}

/// converts 2 byte arrays (U256) into a point on a field
/// exits with OutOfGas for any failed operation
fn uint_to_point(x: U256, y: U256) -> Result<G1, StatusCode> {
    let x = Fq::from_u256(x.0.into()).map_err(|_| OOG)?;
    let y = Fq::from_u256(y.0.into()).map_err(|_| OOG)?;

    Ok(if x.is_zero() && y.is_zero() {
        G1::zero()
    } else {
        AffineG1::new(x, y).map_err(|_| OOG)?.into()
    })
}

/// add 2 points together on `alt_bn128`
fn ec_add(input: &[u8]) -> PrecompileResult {
    let x1 = read_u256(input, 0, OOG)?;
    let y1 = read_u256(input, 32, OOG)?;
    let point1 = uint_to_point(x1, y1)?;

    let x2 = read_u256(input, 64, OOG)?;
    let y2 = read_u256(input, 96, OOG)?;
    let point2 = uint_to_point(x2, y2)?;

    let output = AffineG1::from_jacobian(point1 + point2).map_or(vec![0; 64], |sum| {
        let mut output = vec![0; 64];
        sum.x().to_big_endian(&mut output[..32]).unwrap();
        sum.y().to_big_endian(&mut output[32..]).unwrap();
        output
    });

    Ok(output)
}

/// multiply a scalar and a point on `alt_bn128`
fn ec_mul(input: &[u8]) -> PrecompileResult {
    let mut cost = 6_000; // TODO consume all gas on any op fail

    let x = read_u256(input, 0, OOG)?;
    let y = read_u256(input, 32, OOG)?;
    let point = uint_to_point(x, y)?;

    let scalar = Fr::from_slice(&input[64..95]).map_err(|_| OOG)?;

    let mut output = vec![0; 64];
    if let Some(product) = AffineG1::from_jacobian(point.mul(scalar)) {
        product.x().to_big_endian(&mut output[..32]).unwrap();
        product.y().to_big_endian(&mut output[32..]).unwrap();
    }

    Ok(output)
}

fn ecpairing(input: &[u8]) -> PrecompileResult {

    todo!()
}

// https://eips.ethereum.org/EIPS/eip-152
fn blake2f(input: &[u8]) -> PrecompileResult {
    const GFROUND: u64 = 1;
    let mut hasher = near_blake2::VarBlake2b::default();

    let mut rounds = [0u8; 4];
    let mut h = [0u8; 64];
    let mut m = [0u8; 128];
    let mut t = [0u8; 16];

    // TODO bounds check maybe?
    let mut start = 0;
    rounds.copy_from_slice(&input[..4]);
    start += 4;
    h.copy_from_slice(&input[start..start + 64]);
    start += 64;
    m.copy_from_slice(&input[start..start + 128]);
    start += 128;
    t.copy_from_slice(&input[start..start + 16]);
    start += 16;
    let f = input[start] != 0;

    let rounds = u32::from_be_bytes(rounds);
    // SAFETY: assumes runtime is Little Endian
    let h: [u64; 8] = unsafe { transmute(h) };
    let m: [u64; 16] = unsafe { transmute(m) };
    let t: [u64; 2] = unsafe { transmute(t) };

    let cost = GFROUND * rounds as u64;

    // TODO gas failure
    hasher.blake2_f(rounds, h, m, t, f);
    let output = hasher.output().to_vec();
    Ok(output)
}

/// List of precompile smart contracts, index + 1 is the address (another option is to make an enum)
pub const PRECOMPILES: [PrecompileFn; 9] = [
    ec_recover, // ecrecover 0x01
    sha256,     // SHA256 (Keccak) 0x02
    ripemd160,  // ripemd160 0x03
    identity,   // identity 0x04
    modexp,     // modexp 0x05
    ec_add,     // ecAdd 0x06
    ec_mul,     // ecMul 0x07
    nop,        // ecPairing 0x08
    blake2f,    // blake2f 0x09
];

pub const MAX_PRECOMPILE: H160 = {
    let mut bytes = [0u8; 20];
    bytes[0] = PRECOMPILES.len() as u8;
    H160(bytes)
};

pub fn call_precompile(precompile_addr: H160, input: &[u8]) -> PrecompileResult {
    PRECOMPILES[precompile_addr.0[0] as usize - 1](input)
}
