use std::{marker::PhantomData, ops::Mul};

use super::U256;
use fil_actors_runtime::runtime::{Primitives, Runtime};
use fvm_ipld_blockstore::Blockstore;
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

#[derive(Debug, Eq, PartialEq)]
pub enum PrecompileError {
    EcErr,
    IncorrectInputSize,
}

pub type PrecompileFn<RT> = fn(&RT, &[u8]) -> PrecompileResult;
pub type PrecompileResult = Result<Vec<u8>, PrecompileError>; // TODO i dont like vec

/// Generates a list of precompile smart contracts, index + 1 is the address (another option is to make an enum)
const fn gen_precompiles<RT: Primitives>() -> [PrecompileFn<RT>; 9] {
    [
        ec_recover, // ecrecover 0x01
        sha256,     // SHA2-256 0x02
        ripemd160,  // ripemd160 0x03
        identity,   // identity 0x04
        modexp,     // modexp 0x05
        ec_add,     // ecAdd 0x06
        ec_mul,     // ecMul 0x07
        ec_pairing, // ecPairing 0x08
        blake2f,    // blake2f 0x09
    ]
}

pub struct Precompiles<BS, RT>(PhantomData<BS>, PhantomData<RT>);

impl<BS: Blockstore, RT: Runtime<BS>> Precompiles<BS, RT> {
    const PRECOMPILES: [PrecompileFn<RT>; 9] = gen_precompiles();
    const MAX_PRECOMPILE: U256 = {
        let mut limbs = [0u64; 4];
        limbs[0] = Self::PRECOMPILES.len() as u64;
        U256(limbs)
    };

    pub fn call_precompile(runtime: &RT, precompile_addr: U256, input: &[u8]) -> PrecompileResult {
        Self::PRECOMPILES[precompile_addr.0[0] as usize - 1](runtime, input)
    }

    pub fn is_precompile(addr: &U256) -> bool {
        !addr.is_zero() && addr <= &Self::MAX_PRECOMPILE
    }
}

/// read 32 bytes (u256) from buffer or error
fn read_u256(buf: &[u8], start: usize) -> Result<U256, PrecompileError> {
    let slice = buf.get(start..start + 32).ok_or(PrecompileError::IncorrectInputSize)?;
    Ok(U256::from_big_endian(slice))
}

/// read 32 bytes (u256) from buffer, pass in exit reason that is desired
/// returns 0 if failed to read
fn read_u256_infalliable(buf: &[u8], start: usize) -> U256 {
    let slice = buf.get(start..start + 32).unwrap_or(&[0u8; 32]);
    U256::from_big_endian(slice)
}

fn ec_recover<RT: Primitives>(rt: &RT, input: &[u8]) -> PrecompileResult {
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

    let recovered = rt.recover_secp_public_key(&hash, &sig).unwrap_or([0u8; SECP_PUB_LEN]);

    Ok(recovered.to_vec())
}

fn sha256<RT: Primitives>(rt: &RT, input: &[u8]) -> PrecompileResult {
    Ok(rt.hash(SupportedHashes::Sha2_256, input))
}

fn ripemd160<RT: Primitives>(rt: &RT, input: &[u8]) -> PrecompileResult {
    Ok(rt.hash(SupportedHashes::Ripemd160, input))
}

fn identity<RT: Primitives>(_: &RT, input: &[u8]) -> PrecompileResult {
    Ok(Vec::from(input))
}

// https://eips.ethereum.org/EIPS/eip-198
fn modexp<RT: Primitives>(_: &RT, input: &[u8]) -> PrecompileResult {
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
fn ec_add<RT: Primitives>(_: &RT, input: &[u8]) -> PrecompileResult {
    let x1 = read_u256_infalliable(input, 0);
    let y1 = read_u256_infalliable(input, 32);
    let point1 = uint_to_point(x1, y1)?;

    let x2 = read_u256_infalliable(input, 64);
    let y2 = read_u256_infalliable(input, 96);
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
fn ec_mul<RT: Primitives>(_: &RT, input: &[u8]) -> PrecompileResult {
    let x = read_u256_infalliable(input, 0);
    let y = read_u256_infalliable(input, 32);
    let point = uint_to_point(x, y)?;

    let scalar = if let Some(scalar) = input.get(64..96) {
        Fr::from_slice(scalar).map_err(|_| PrecompileError::EcErr)?
    } else {
        return Ok(vec![0; 64]);
    };

    AffineG1::from_jacobian(point.mul(scalar)).ok_or(PrecompileError::EcErr).map(|product| {
        let mut output = vec![0; 64];
        product.x().to_big_endian(&mut output[..32]).unwrap();
        product.y().to_big_endian(&mut output[32..]).unwrap();
        output
    })
}

fn ec_pairing<RT: Primitives>(_: &RT, input: &[u8]) -> PrecompileResult {
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
fn blake2f<RT: Primitives>(_: &RT, input: &[u8]) -> PrecompileResult {
    if input.len() != 213 {
        return Err(PrecompileError::IncorrectInputSize);
    }
    let mut hasher = near_blake2::VarBlake2b::default();

    let mut rounds = [0u8; 4];

    // 4 bytes
    let mut start = 0;
    rounds.copy_from_slice(&input[..4]);
    start += 4;

    // 64 bytes
    let h = &input[start..start + 64];
    start += 64;
    // 128 bytes
    let m = &input[start..start + 128];
    start += 128;
    // 16 bytes
    let t = &input[start..start + 16];
    start += 16;

    debug_assert_eq!(start, 212, "expected start to be at the last byte");
    let f = match input[start] {
        0 => Ok(false),
        1 => Ok(true),
        _ => Err(PrecompileError::IncorrectInputSize),
    }?;

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

#[cfg(test)]
mod tests {
    use super::*;
    use fil_actors_runtime::test_utils::MockRuntime;
    use hex_literal::hex;

    #[test]
    fn sha256() {
        use super::sha256 as hash;
        let input = "foo bar baz boxy".as_bytes();

        let rt = MockRuntime::default();

        let expected = hex!("ace8597929092c14bd028ede7b07727875788c7e130278b5afed41940d965aba");
        let res = hash(&rt, input).unwrap();
        assert_eq!(&res, &expected);
    }

    #[test]
    fn ripemd160() {
        use super::ripemd160 as hash;
        let input = "foo bar baz boxy".as_bytes();

        let rt = MockRuntime::default();

        let expected = hex!("4cd7a0452bd3d682e4cbd5fa90f446d7285b156a");
        let res = hash(&rt, input).unwrap();
        assert_eq!(&res, &expected);
    }

    // bn tests borrowed from https://github.com/bluealloy/revm/blob/26540bf5b29de6e7c8020c4c1880f8a97d1eadc9/crates/revm_precompiles/src/bn128.rs
    mod bn {
        use super::MockRuntime;
        use crate::interpreter::precompiles::{ec_add, ec_mul, PrecompileError};

        #[test]
        fn bn128_add() {
            let rt = MockRuntime::default();

            let input = hex::decode(
                "\
                 18b18acfb4c2c30276db5411368e7185b311dd124691610c5d3b74034e093dc9\
                 063c909c4720840cb5134cb9f59fa749755796819658d32efc0d288198f37266\
                 07c2b7f58a84bd6145f00c9c2bc0bb1a187f20ff2c92963a88019e7c6a014eed\
                 06614e20c147e940f2d70da3f74c9a17df361706a4485c742bd6788478fa17d7",
            )
            .unwrap();
            let expected = hex::decode(
                "\
                2243525c5efd4b9c3d3c45ac0ca3fe4dd85e830a4ce6b65fa1eeaee202839703\
                301d1d33be6da8e509df21cc35964723180eed7532537db9ae5e7d48f195c915",
            )
            .unwrap();
            let res = ec_add(&rt, &input).unwrap();
            assert_eq!(res, expected);
            // zero sum test
            let input = hex::decode(
                "\
                0000000000000000000000000000000000000000000000000000000000000000\
                0000000000000000000000000000000000000000000000000000000000000000\
                0000000000000000000000000000000000000000000000000000000000000000\
                0000000000000000000000000000000000000000000000000000000000000000",
            )
            .unwrap();
            let expected = hex::decode(
                "\
                0000000000000000000000000000000000000000000000000000000000000000\
                0000000000000000000000000000000000000000000000000000000000000000",
            )
            .unwrap();
            let res = ec_add(&rt, &input).unwrap();
            assert_eq!(res, expected);

            // no input test
            let input = [];
            let expected = hex::decode(
                "\
                0000000000000000000000000000000000000000000000000000000000000000\
                0000000000000000000000000000000000000000000000000000000000000000",
            )
            .unwrap();
            let res = ec_add(&rt, &input).unwrap();
            assert_eq!(res, expected);
            // point not on curve fail
            let input = hex::decode(
                "\
                1111111111111111111111111111111111111111111111111111111111111111\
                1111111111111111111111111111111111111111111111111111111111111111\
                1111111111111111111111111111111111111111111111111111111111111111\
                1111111111111111111111111111111111111111111111111111111111111111",
            )
            .unwrap();
            let res = ec_add(&rt, &input);
            assert!(matches!(res, Err(PrecompileError::EcErr)));
        }

        #[test]
        fn bn128_mul() {
            let rt = MockRuntime::default();

            let input = hex::decode(
                "\
                2bd3e6d0f3b142924f5ca7b49ce5b9d54c4703d7ae5648e61d02268b1a0a9fb7\
                21611ce0a6af85915e2f1d70300909ce2e49dfad4a4619c8390cae66cefdb204\
                00000000000000000000000000000000000000000000000011138ce750fa15c2",
            )
            .unwrap();
            let expected = hex::decode(
                "\
                070a8d6a982153cae4be29d434e8faef8a47b274a053f5a4ee2a6c9c13c31e5c\
                031b8ce914eba3a9ffb989f9cdd5b0f01943074bf4f0f315690ec3cec6981afc",
            )
            .unwrap();
            let res = ec_mul(&rt, &input).unwrap();
            assert_eq!(res, expected);

            // out of gas test
            let input = hex::decode(
                "\
                0000000000000000000000000000000000000000000000000000000000000000\
                0000000000000000000000000000000000000000000000000000000000000000\
                0200000000000000000000000000000000000000000000000000000000000000",
            )
            .unwrap();
            let res = ec_mul(&rt, &input);
            assert_eq!(res, Err(PrecompileError::EcErr));

            // no input test
            let input = [0u8; 0];
            let expected = hex::decode(
                "\
                0000000000000000000000000000000000000000000000000000000000000000\
                0000000000000000000000000000000000000000000000000000000000000000",
            )
            .unwrap();
            let res = ec_mul(&rt, &input).unwrap();
            assert_eq!(res, expected);
            // point not on curve fail
            let input = hex::decode(
                "\
                1111111111111111111111111111111111111111111111111111111111111111\
                1111111111111111111111111111111111111111111111111111111111111111\
                0f00000000000000000000000000000000000000000000000000000000000000",
            )
            .unwrap();
            let res = ec_mul(&rt, &input);
            assert!(matches!(res, Err(PrecompileError::EcErr)));
        }

        // #[test]
        // fn bn128_pair() {
        //     let input = hex::decode(
        //         "\
        //         1c76476f4def4bb94541d57ebba1193381ffa7aa76ada664dd31c16024c43f59\
        //         3034dd2920f673e204fee2811c678745fc819b55d3e9d294e45c9b03a76aef41\
        //         209dd15ebff5d46c4bd888e51a93cf99a7329636c63514396b4a452003a35bf7\
        //         04bf11ca01483bfa8b34b43561848d28905960114c8ac04049af4b6315a41678\
        //         2bb8324af6cfc93537a2ad1a445cfd0ca2a71acd7ac41fadbf933c2a51be344d\
        //         120a2a4cf30c1bf9845f20c6fe39e07ea2cce61f0c9bb048165fe5e4de877550\
        //         111e129f1cf1097710d41c4ac70fcdfa5ba2023c6ff1cbeac322de49d1b6df7c\
        //         2032c61a830e3c17286de9462bf242fca2883585b93870a73853face6a6bf411\
        //         198e9393920d483a7260bfb731fb5d25f1aa493335a9e71297e485b7aef312c2\
        //         1800deef121f1e76426a00665e5c4479674322d4f75edadd46debd5cd992f6ed\
        //         090689d0585ff075ec9e99ad690c3395bc4b313370b38ef355acdadcd122975b\
        //         12c85ea5db8c6deb4aab71808dcb408fe3d1e7690c43d37b4ce6cc0166fa7daa",
        //     )
        //     .unwrap();
        //     let expected =
        //         hex::decode("0000000000000000000000000000000000000000000000000000000000000001")
        //             .unwrap();
        //     let res =
        //         Bn128Pair::<Byzantium>::run(&input, 260_000, &new_context(), false).unwrap().output;
        //     assert_eq!(res, expected);
        //     // out of gas test
        //     let input = hex::decode(
        //         "\
        //         1c76476f4def4bb94541d57ebba1193381ffa7aa76ada664dd31c16024c43f59\
        //         3034dd2920f673e204fee2811c678745fc819b55d3e9d294e45c9b03a76aef41\
        //         209dd15ebff5d46c4bd888e51a93cf99a7329636c63514396b4a452003a35bf7\
        //         04bf11ca01483bfa8b34b43561848d28905960114c8ac04049af4b6315a41678\
        //         2bb8324af6cfc93537a2ad1a445cfd0ca2a71acd7ac41fadbf933c2a51be344d\
        //         120a2a4cf30c1bf9845f20c6fe39e07ea2cce61f0c9bb048165fe5e4de877550\
        //         111e129f1cf1097710d41c4ac70fcdfa5ba2023c6ff1cbeac322de49d1b6df7c\
        //         2032c61a830e3c17286de9462bf242fca2883585b93870a73853face6a6bf411\
        //         198e9393920d483a7260bfb731fb5d25f1aa493335a9e71297e485b7aef312c2\
        //         1800deef121f1e76426a00665e5c4479674322d4f75edadd46debd5cd992f6ed\
        //         090689d0585ff075ec9e99ad690c3395bc4b313370b38ef355acdadcd122975b\
        //         12c85ea5db8c6deb4aab71808dcb408fe3d1e7690c43d37b4ce6cc0166fa7daa",
        //     )
        //     .unwrap();
        //     let res = Bn128Pair::<Byzantium>::run(&input, 259_999, &new_context(), false);
        //     assert!(matches!(res, Err(Return::OutOfGas)));
        //     // no input test
        //     let input = [0u8; 0];
        //     let expected =
        //         hex::decode("0000000000000000000000000000000000000000000000000000000000000001")
        //             .unwrap();
        //     let res =
        //         Bn128Pair::<Byzantium>::run(&input, 260_000, &new_context(), false).unwrap().output;
        //     assert_eq!(res, expected);
        //     // point not on curve fail
        //     let input = hex::decode(
        //         "\
        //         1111111111111111111111111111111111111111111111111111111111111111\
        //         1111111111111111111111111111111111111111111111111111111111111111\
        //         1111111111111111111111111111111111111111111111111111111111111111\
        //         1111111111111111111111111111111111111111111111111111111111111111\
        //         1111111111111111111111111111111111111111111111111111111111111111\
        //         1111111111111111111111111111111111111111111111111111111111111111",
        //     )
        //     .unwrap();
        //     let res = Bn128Pair::<Byzantium>::run(&input, 260_000, &new_context(), false);
        //     assert!(matches!(res, Err(Return::Other(Cow::Borrowed("ERR_BN128_INVALID_A")))));
        //     // invalid input length
        //     let input = hex::decode(
        //         "\
        //         1111111111111111111111111111111111111111111111111111111111111111\
        //         1111111111111111111111111111111111111111111111111111111111111111\
        //         111111111111111111111111111111\
        //     ",
        //     )
        //     .unwrap();
        //     let res = Bn128Pair::<Byzantium>::run(&input, 260_000, &new_context(), false);
        //     assert!(matches!(res, Err(Return::Other(Cow::Borrowed("ERR_BN128_INVALID_LEN",)))));
        // }
    }

    // https://eips.ethereum.org/EIPS/eip-152#test-cases
    #[test]
    fn blake2() {
        use super::blake2f;
        let rt = MockRuntime::default();

        // // helper to turn EIP test cases into something readable
        // fn test_case_formatter(mut remaining: impl ToString) {
        //     let mut rounds = remaining.to_string();
        //     let mut h = rounds.split_off(2*4);
        //     let mut m = h.split_off(2*64);
        //     let mut t_0 = m.split_off(2*128);
        //     let mut t_1 = t_0.split_off(2*8);
        //     let mut f = t_1.split_off(2*8);

        //     println!("
        //         \"{rounds}\"
        //         \"{h}\"
        //         \"{m}\"
        //         \"{t_0}\"
        //         \"{t_1}\"
        //         \"{f}\"
        //     ")
        // }

        // T0 invalid input len
        assert_eq!(blake2f(&rt, &[]), Err(PrecompileError::IncorrectInputSize));

        // T1 too small
        let input = &hex!(
            "00000c"
            "48c9bdf267e6096a3ba7ca8485ae67bb2bf894fe72f36e3cf1361d5f3af54fa5d182e6ad7f520e511f6c3e2b8c68059b6bbd41fbabd9831f79217e1319cde05b"
            "6162630000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000"
            "0300000000000000"
            "0000000000000000"
            "02"
        );
        assert_eq!(blake2f(&rt, input), Err(PrecompileError::IncorrectInputSize));

        // T2 too large
        let input = &hex!(
            "000000000c"
            "48c9bdf267e6096a3ba7ca8485ae67bb2bf894fe72f36e3cf1361d5f3af54fa5d182e6ad7f520e511f6c3e2b8c68059b6bbd41fbabd9831f79217e1319cde05b"
            "6162630000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000"
            "0300000000000000"
            "0000000000000000"
            "02"
        );
        assert_eq!(blake2f(&rt, input), Err(PrecompileError::IncorrectInputSize));

        // T3 final block indicator invalid
        let input = &hex!(
            "0000000c"
            "48c9bdf267e6096a3ba7ca8485ae67bb2bf894fe72f36e3cf1361d5f3af54fa5d182e6ad7f520e511f6c3e2b8c68059b6bbd41fbabd9831f79217e1319cde05b"
            "6162630000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000"
            "0300000000000000"
            "0000000000000000"
            "02"
        );
        assert_eq!(blake2f(&rt, input), Err(PrecompileError::IncorrectInputSize));

        // outputs

        // T4
        let expected = &hex!("08c9bcf367e6096a3ba7ca8485ae67bb2bf894fe72f36e3cf1361d5f3af54fa5d282e6ad7f520e511f6c3e2b8c68059b9442be0454267ce079217e1319cde05b");
        let input = &hex!(
            "00000000"
            "48c9bdf267e6096a3ba7ca8485ae67bb2bf894fe72f36e3cf1361d5f3af54fa5d182e6ad7f520e511f6c3e2b8c68059b6bbd41fbabd9831f79217e1319cde05b"
            "6162630000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000"
            "0300000000000000"
            "0000000000000000"
            "01"
        );
        assert_eq!(blake2f(&rt, input), Ok(expected.to_vec()));

        // T5
        let expected = &hex!("ba80a53f981c4d0d6a2797b69f12f6e94c212f14685ac4b74b12bb6fdbffa2d17d87c5392aab792dc252d5de4533cc9518d38aa8dbf1925ab92386edd4009923");
        let input = &hex!(
            "0000000c"
            "48c9bdf267e6096a3ba7ca8485ae67bb2bf894fe72f36e3cf1361d5f3af54fa5d182e6ad7f520e511f6c3e2b8c68059b6bbd41fbabd9831f79217e1319cde05b"
            "6162630000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000"
            "0300000000000000"
            "0000000000000000"
            "01"
        );
        assert_eq!(blake2f(&rt, input), Ok(expected.to_vec()));

        // T6
        let expected = &hex!("75ab69d3190a562c51aef8d88f1c2775876944407270c42c9844252c26d2875298743e7f6d5ea2f2d3e8d226039cd31b4e426ac4f2d3d666a610c2116fde4735");
        let input = &hex!(
            "0000000c"
            "48c9bdf267e6096a3ba7ca8485ae67bb2bf894fe72f36e3cf1361d5f3af54fa5d182e6ad7f520e511f6c3e2b8c68059b6bbd41fbabd9831f79217e1319cde05b"
            "6162630000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000"
            "0300000000000000"
            "0000000000000000"
            "00"
        );
        assert_eq!(blake2f(&rt, input), Ok(expected.to_vec()));

        // T7
        let expected = &hex!("b63a380cb2897d521994a85234ee2c181b5f844d2c624c002677e9703449d2fba551b3a8333bcdf5f2f7e08993d53923de3d64fcc68c034e717b9293fed7a421");
        let input = &hex!(
            "00000001"
            "48c9bdf267e6096a3ba7ca8485ae67bb2bf894fe72f36e3cf1361d5f3af54fa5d182e6ad7f520e511f6c3e2b8c68059b6bbd41fbabd9831f79217e1319cde05b"
            "6162630000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000"
            "0300000000000000"
            "0000000000000000"
            "01"
        );
        assert_eq!(blake2f(&rt, input), Ok(expected.to_vec()));

        // T8
        // NOTE:
        // original test case ran ffffffff rounds of blake2b
        // with an output of fc59093aafa9ab43daae0e914c57635c5402d8e3d2130eb9b3cc181de7f0ecf9b22bf99a7815ce16419e200e01846e6b5df8cc7703041bbceb571de6631d2615
        // I ran this sucessfully while grabbing a cup of coffee, so if you fee like wasting u32::MAX rounds of hash time (25-ish min on Ryzen5 2600) you can test it as such
        // For my and CI's sanity however, we are capping it at 0000ffff.
        let expected = &hex!("183ed9b1e5594bcdd715a4e4fd7b0dc2eaa2ef9bda48242af64c687081142156621bc94bb2d5aa99d83c2f1a5d9c426e1b6a1755a5e080f6217e2a5f3b9c4624");
        let input = &hex!(
            "0000ffff"
            "48c9bdf267e6096a3ba7ca8485ae67bb2bf894fe72f36e3cf1361d5f3af54fa5d182e6ad7f520e511f6c3e2b8c68059b6bbd41fbabd9831f79217e1319cde05b"
            "6162630000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000"
            "0300000000000000"
            "0000000000000000"
            "01"
        );
        assert_eq!(blake2f(&rt, input), Ok(expected.to_vec()));
    }
}
