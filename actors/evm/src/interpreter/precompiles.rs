use std::ops::Mul;

use super::{H160, U256};
use fvm_shared::{
    bigint::BigUint,
    crypto::{
        hash::SupportedHashes,
        signature::{SECP_PUB_LEN, SECP_SIG_LEN, SECP_SIG_MESSAGE_HASH_SIZE},
    },
};
use num_traits::{One, Zero};
use substrate_bn::{pairing_batch, AffineG1, AffineG2, Fq, Fq2, Fr, Group, Gt, G1, G2};
use uint::byteorder::{ByteOrder, LE};

pub fn is_precompile(addr: &H160) -> bool {
    !addr.is_zero() && addr <= &MAX_PRECOMPILE
}

/// read 32 bytes (u256) from buffer, pass in exit reason that is desired
/// TODO passing in err value is debatable
fn read_u256(buf: &[u8], start: usize) -> Result<U256, PrecompileError> {
    let slice = buf.get(start..start + 32).ok_or(PrecompileError::IncorrectInputSize)?;
    Ok(U256::from_big_endian(slice))
}

#[derive(Debug)]
pub enum PrecompileError {
    EcErr,
    IncorrectInputSize,
}

pub type PrecompileFn = fn(&[u8]) -> PrecompileResult;
pub type PrecompileResult = Result<Vec<u8>, PrecompileError>; // TODO i dont like vec

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

// https://eips.ethereum.org/EIPS/eip-198
fn modexp(input: &[u8]) -> PrecompileResult {
    let base_len = read_u256(input, 0)?.as_usize();
    let exponent_len = read_u256(input, 32)?.as_usize();
    let mod_len = read_u256(input, 64)?.as_usize();

    if mod_len == 0 {
        return Ok(Vec::new());
    }

    // TODO bounds checking
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
fn uint_to_point(x: U256, y: U256) -> Result<G1, PrecompileError> {
    let x = Fq::from_u256(x.0.into()).map_err(|_| PrecompileError::EcErr)?;
    let y = Fq::from_u256(y.0.into()).map_err(|_| PrecompileError::EcErr)?;

    Ok(if x.is_zero() && y.is_zero() {
        G1::zero()
    } else {
        AffineG1::new(x, y).map_err(|_| PrecompileError::EcErr)?.into()
    })
}

/// add 2 points together on `alt_bn128`
fn ec_add(input: &[u8]) -> PrecompileResult {
    let x1 = read_u256(input, 0)?;
    let y1 = read_u256(input, 32)?;
    let point1 = uint_to_point(x1, y1)?;

    let x2 = read_u256(input, 64)?;
    let y2 = read_u256(input, 96)?;
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
    let x = read_u256(input, 0)?;
    let y = read_u256(input, 32)?;
    let point = uint_to_point(x, y)?;

    let scalar = Fr::from_slice(&input[64..95]).map_err(|_| PrecompileError::EcErr)?;

    let mut output = vec![0; 64];
    if let Some(product) = AffineG1::from_jacobian(point.mul(scalar)) {
        product.x().to_big_endian(&mut output[..32]).unwrap();
        product.y().to_big_endian(&mut output[32..]).unwrap();
    }

    Ok(output)
}

fn ecpairing(input: &[u8]) -> PrecompileResult {
    fn read_group(input: &[u8]) -> Result<(G1, G2), PrecompileError> {
        let x1 = read_u256(input, 0)?;
        let y1 = read_u256(input, 32)?;

        let y2 = read_u256(input, 64)?;
        let x2 = read_u256(input, 96)?;
        let y3 = read_u256(input, 128)?;
        let x3 = read_u256(input, 160)?;

        // TODO errs
        let ax = Fq::from_u256(x1.0.into()).unwrap();
        let ay = Fq::from_u256(y1.0.into()).unwrap();

        let twisted_ax = Fq::from_u256(x2.0.into()).unwrap();
        let twisted_ay = Fq::from_u256(y2.0.into()).unwrap();
        let twisted_bx = Fq::from_u256(x3.0.into()).unwrap();
        let twisted_by = Fq::from_u256(y3.0.into()).unwrap();

        let twisted_a = Fq2::new(twisted_ax, twisted_ay);
        let twisted_b = Fq2::new(twisted_bx, twisted_by);

        let twisted = {
            if twisted_a.is_zero() && twisted_b.is_zero() {
                G2::zero()
            } else {
                AffineG2::new(twisted_a, twisted_b).unwrap().into()
            }
        };

        let a = {
            if ax.is_zero() && ay.is_zero() {
                substrate_bn::G1::zero()
            } else {
                AffineG1::new(ax, ay).unwrap().into()
            }
        };

        Ok((a, twisted))
    }

    const GROUP_BYTE_LEN: usize = 192;

    if input.len() % GROUP_BYTE_LEN != 0 {
        return Err(PrecompileError::IncorrectInputSize);
    }

    let mut groups = Vec::new();
    for i in 0..input.len() / GROUP_BYTE_LEN {
        let offset = i * GROUP_BYTE_LEN;
        groups.push(read_group(&input[offset..offset + GROUP_BYTE_LEN])?);
    }

    let accumulated = pairing_batch(&groups);

    let output = if accumulated == Gt::one() { U256::one() } else { U256::zero() };
    let mut buf = [0u8; 32];
    output.to_big_endian(&mut buf);
    Ok(buf.to_vec())
}

// https://eips.ethereum.org/EIPS/eip-152
fn blake2f(input: &[u8]) -> PrecompileResult {
    if input.len() != 213 {
        return Err(PrecompileError::IncorrectInputSize);
    }
    let mut hasher = near_blake2::VarBlake2b::default();

    let mut rounds = [0u8; 4];

    let mut start = 0;
    rounds.copy_from_slice(&input[..4]);
    start += 4;

    let h = &input[start..start + 64];
    start += 64;
    let m = &input[start..start + 128];
    start += 128;
    let t = &input[start..start + 16];
    start += 16;
    let f = input[start] != 0;

    let rounds = u32::from_be_bytes(rounds);
    let h = {
        let mut ret = [0u64; 8];
        LE::read_u64_into(h, &mut ret);
        ret
    };
    let m = {
        let mut ret = [0u64; 16];
        LE::read_u64_into(m, &mut ret);
        ret
    };
    let t = {
        let mut ret = [0u64; 2];
        LE::read_u64_into(t, &mut ret);
        ret
    };

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
    ecpairing,  // ecPairing 0x08
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
