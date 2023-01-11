#![allow(dead_code)]
// to silence construct_uint! clippy warnings
// see https://github.com/paritytech/parity-common/issues/660
#![allow(clippy::ptr_offset_with_cast, clippy::assign_op_pattern)]

use serde::{Deserialize, Serialize};
use substrate_bn::arith;

use crate::BytecodeHash;

use {
    fvm_shared::bigint::BigInt, fvm_shared::econ::TokenAmount, std::cmp::Ordering, std::fmt,
    uint::construct_uint,
};

construct_uint! { pub struct U256(4); } // ethereum word size
construct_uint! { pub struct U512(8); } // used for addmod and mulmod opcodes

// Convenience method for comparing against a small value.
impl PartialOrd<u64> for U256 {
    fn partial_cmp(&self, other: &u64) -> Option<Ordering> {
        if self.0[3] > 0 || self.0[2] > 0 || self.0[1] > 0 {
            Some(Ordering::Greater)
        } else {
            self.0[0].partial_cmp(other)
        }
    }
}

impl PartialEq<u64> for U256 {
    fn eq(&self, other: &u64) -> bool {
        self.0[0] == *other && self.0[1] == 0 && self.0[2] == 0 && self.0[3] == 0
    }
}

impl U256 {
    pub const BITS: u32 = 256;
    pub const ZERO: Self = U256::from_u64(0);
    pub const ONE: Self = U256::from_u64(1);
    pub const I128_MIN: Self = U256([0, 0, 0, i64::MIN as u64]);

    #[inline(always)]
    pub const fn from_u128_words(high: u128, low: u128) -> U256 {
        U256([low as u64, (low >> u64::BITS) as u64, high as u64, (high >> u64::BITS) as u64])
    }

    #[inline(always)]
    pub const fn from_u64(value: u64) -> U256 {
        U256([value, 0, 0, 0])
    }

    #[inline(always)]
    pub const fn i256_is_negative(&self) -> bool {
        (self.0[3] as i64) < 0
    }

    /// turns a i256 value to negative
    #[inline(always)]
    pub fn i256_neg(&self) -> U256 {
        !*self + U256::ONE
    }

    pub fn to_bytes(&self) -> [u8; 32] {
        let mut buf = [0u8; 32];
        self.to_big_endian(&mut buf);
        buf
    }

    /// Returns the low 64 bits, saturating the value to u64 max if it is larger
    pub fn to_u64_saturating(&self) -> u64 {
        if self.bits() > 64 {
            u64::MAX
        } else {
            self.0[0]
        }
    }
}

impl U512 {
    pub fn low_u256(&self) -> U256 {
        let [a, b, c, d, ..] = self.0;
        U256([a, b, c, d])
    }
}

impl From<&TokenAmount> for U256 {
    fn from(amount: &TokenAmount) -> U256 {
        let (_, bytes) = amount.atto().to_bytes_be();
        U256::from(bytes.as_slice())
    }
}

impl From<U256> for arith::U256 {
    fn from(src: U256) -> arith::U256 {
        arith::U256::from(src.0)
    }
}

impl From<U256> for U512 {
    fn from(v: U256) -> Self {
        let [a, b, c, d] = v.0;
        U512([a, b, c, d, 0, 0, 0, 0])
    }
}

impl From<&U256> for TokenAmount {
    fn from(ui: &U256) -> TokenAmount {
        let mut bits = [0u8; 32];
        ui.to_big_endian(&mut bits);
        TokenAmount::from_atto(BigInt::from_bytes_be(fvm_shared::bigint::Sign::Plus, &bits))
    }
}

impl From<BytecodeHash> for U256 {
    fn from(bytecode: BytecodeHash) -> Self {
        let bytes: [u8; 32] = bytecode.into();
        Self::from(bytes)
    }
}

impl Serialize for U256 {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        let mut bytes = [0u8; 32];
        self.to_big_endian(&mut bytes);
        serializer.serialize_bytes(zeroless_view(&bytes))
    }
}

impl<'de> Deserialize<'de> for U256 {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        struct Visitor;
        impl<'de> serde::de::Visitor<'de> for Visitor {
            type Value = U256;

            fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
                write!(formatter, "at most 32 bytes")
            }

            fn visit_bytes<E>(self, v: &[u8]) -> Result<Self::Value, E>
            where
                E: serde::de::Error,
            {
                if v.len() > 32 {
                    return Err(serde::de::Error::invalid_length(v.len(), &self));
                }
                Ok(U256::from_big_endian(v))
            }
        }
        deserializer.deserialize_bytes(Visitor)
    }
}

fn zeroless_view(v: &impl AsRef<[u8]>) -> &[u8] {
    let v = v.as_ref();
    &v[v.iter().take_while(|&&b| b == 0).count()..]
}

macro_rules! impl_rlp_codec_uint {
  ($type:ident, $bytes_len: expr) => {
    impl rlp::Encodable for $type {
      fn rlp_append(&self, s: &mut rlp::RlpStream) {
        let mut bytes = [0u8; $bytes_len];
        self.to_big_endian(&mut bytes);
        let zbytes = zeroless_view(&bytes);
        s.encoder().encode_value(&zbytes);
      }
    }
    impl rlp::Decodable for $type {
      fn decode(rlp: &rlp::Rlp) -> Result<Self, rlp::DecoderError> {
        rlp
          .decoder()
          .decode_value(|bytes| Ok($type::from_big_endian(bytes)))
      }
    }
  };
}

// RLP Support
impl_rlp_codec_uint!(U256, 32);
impl_rlp_codec_uint!(U512, 64);

#[inline(always)]
pub fn i256_div(mut first: U256, mut second: U256) -> U256 {
    if first.is_zero() || second.is_zero() {
        // EVM defines X/0 to be 0.
        return U256::ZERO;
    }

    // min-negative-value can't be represented as a positive value, but we don't need to.
    // NOTE: we've already checked that 'second' isn't zero above.
    if (first, second) == (U256::I128_MIN, U256::ONE) {
        return U256::I128_MIN;
    }

    // Record and strip the signs. We add them back at the end.
    let first_neg = first.i256_is_negative();
    let second_neg = second.i256_is_negative();

    if first_neg {
        first = first.i256_neg()
    }

    if second_neg {
        second = second.i256_neg()
    }

    let d = first / second;

    // Flip the sign back if necessary.
    if d.is_zero() || first_neg == second_neg {
        d
    } else {
        d.i256_neg()
    }
}

#[inline(always)]
pub fn i256_mod(mut first: U256, mut second: U256) -> U256 {
    if first.is_zero() || second.is_zero() {
        // X % 0  or 0 % X is always 0.
        return U256::ZERO;
    }

    // Record and strip the sign.
    let negative = first.i256_is_negative();
    if negative {
        first = first.i256_neg();
    }

    if second.i256_is_negative() {
        second = second.i256_neg()
    }

    let r = first % second;

    // Restore the sign.
    if negative && !r.is_zero() {
        r.i256_neg()
    } else {
        r
    }
}

#[inline(always)]
pub fn i256_cmp(first: U256, second: U256) -> Ordering {
    // true > false:
    // - true < positive:
    match second.i256_is_negative().cmp(&first.i256_is_negative()) {
        Ordering::Equal => first.cmp(&second),
        sign_cmp => sign_cmp,
    }
}

#[cfg(test)]
mod tests {
    use fvm_ipld_encoding::{BytesDe, BytesSer, RawBytes};

    use {super::*, core::num::Wrapping};

    #[test]
    fn div_i256() {
        assert_eq!(Wrapping(i8::MIN) / Wrapping(-1), Wrapping(i8::MIN));
        assert_eq!(i8::MAX / -1, -i8::MAX);

        let zero = U256::ZERO;
        let one = U256::ONE;
        let one_hundred = U256::from(100);
        let fifty = U256::from(50);
        let two = U256::from(2);
        let neg_one_hundred = U256::from(100);
        let minus_one = U256::from(1);
        let max_value = U256::from(2).pow(255.into()) - 1;
        let neg_max_value = U256::from(2).pow(255.into()) - 1;

        assert_eq!(i256_div(U256::I128_MIN, minus_one), U256::I128_MIN);
        assert_eq!(i256_div(U256::I128_MIN, one), U256::I128_MIN);
        assert_eq!(i256_div(one, U256::I128_MIN), zero);
        assert_eq!(i256_div(max_value, one), max_value);
        assert_eq!(i256_div(max_value, minus_one), neg_max_value);
        assert_eq!(i256_div(one_hundred, minus_one), neg_one_hundred);
        assert_eq!(i256_div(one_hundred, two), fifty);

        assert_eq!(i256_div(zero, zero), zero);
        assert_eq!(i256_div(one, zero), zero);
        assert_eq!(i256_div(zero, one), zero);
    }

    #[test]
    fn u256_serde() {
        let encoded = RawBytes::serialize(U256::from(0x4d2)).unwrap();
        let BytesDe(bytes) = encoded.deserialize().unwrap();
        assert_eq!(bytes, &[0x04, 0xd2]);
        let decoded: U256 = encoded.deserialize().unwrap();
        assert_eq!(decoded, 0x4d2);
    }

    #[test]
    fn u256_empty() {
        let encoded = RawBytes::serialize(U256::from(0)).unwrap();
        let BytesDe(bytes) = encoded.deserialize().unwrap();
        assert!(bytes.is_empty());
    }

    #[test]
    fn u256_overflow() {
        let encoded = RawBytes::serialize(BytesSer(&[1; 33])).unwrap();
        encoded.deserialize::<U256>().expect_err("should have failed to decode an over-large u256");
    }
}
