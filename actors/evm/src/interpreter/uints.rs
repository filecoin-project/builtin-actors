#![allow(dead_code)]
// to silence construct_uint! clippy warnings
// see https://github.com/paritytech/parity-common/issues/660
#![allow(clippy::ptr_offset_with_cast, clippy::assign_op_pattern)]

use substrate_bn::arith;

use {
    fvm_shared::bigint::BigInt, fvm_shared::econ::TokenAmount, impl_serde::impl_uint_serde,
    std::cmp::Ordering, uint::construct_uint,
};

construct_uint! { pub struct U256(4); } // ethereum word size
construct_uint! { pub struct U512(8); } // used for addmod and mulmod opcodes

impl From<&TokenAmount> for U256 {
    fn from(amount: &TokenAmount) -> U256 {
        let (_, bytes) = amount.atto().to_bytes_be();
        U256::from(bytes.as_slice())
    }
}

impl Into<arith::U256> for U256 {
    fn into(self) -> arith::U256 {
        arith::U256::from(self.0)
    }
}

impl From<&U256> for TokenAmount {
    fn from(ui: &U256) -> TokenAmount {
        let mut bits = [0u8; 32];
        ui.to_big_endian(&mut bits);
        TokenAmount::from_atto(BigInt::from_bytes_be(fvm_shared::bigint::Sign::Plus, &bits))
    }
}

// make ETH uints serde serializable,
// so it can work with Hamt and other
// IPLD structures seamlessly
impl_uint_serde!(U256, 4);
impl_uint_serde!(U512, 8);

macro_rules! impl_hamt_hash {
    ($type:ident) => {
        impl fvm_ipld_hamt::Hash for $type {
            fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
                self.0.hash(state);
            }
        }
    };
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

// Hamt support
impl_hamt_hash!(U256);
impl_hamt_hash!(U512);

// RLP Support
impl_rlp_codec_uint!(U256, 32);
impl_rlp_codec_uint!(U512, 64);

#[inline(always)]
pub fn u256_high(val: U256) -> u128 {
    let mut bytes = [0u8; 32];
    val.to_big_endian(&mut bytes);
    u128::from_be_bytes(bytes[0..16].try_into().unwrap())
}

#[inline(always)]
pub fn u256_low(val: U256) -> u128 {
    let mut bytes = [0u8; 32];
    val.to_big_endian(&mut bytes);
    u128::from_be_bytes(bytes[16..32].try_into().unwrap())
}

#[inline(always)]
pub fn u128_words_to_u256(high: u128, low: u128) -> U256 {
    let high = high.to_be_bytes();
    let low = low.to_be_bytes();
    let bytes = high.into_iter().chain(low.into_iter()).collect::<Vec<_>>();
    U256::from_big_endian(&bytes)
}

const SIGN_BITMASK_U128: u128 = 0x8000_0000_0000_0000_0000_0000_0000_0000;
const FLIPH_BITMASK_U128: u128 = 0x7FFF_FFFF_FFFF_FFFF_FFFF_FFFF_FFFF_FFFF;

#[derive(Copy, Clone, Eq, PartialEq, Debug)]
pub enum Sign {
    Plus,
    Minus,
    Zero,
}

#[inline]
pub fn log2floor(value: U256) -> u64 {
    debug_assert!(value != U256::zero());
    let mut l: u64 = 256;
    for v in [u256_high(value), u256_low(value)] {
        if v == 0 {
            l -= 128;
        } else {
            l -= v.leading_zeros() as u64;
            if l == 0 {
                return l;
            } else {
                return l - 1;
            }
        }
    }
    l
}

#[inline(always)]
pub fn two_compl(op: U256) -> U256 {
    !op + U256::from(1)
}

#[inline(always)]
fn two_compl_mut(op: &mut U256) {
    *op = two_compl(*op);
}

#[inline(always)]
pub fn i256_sign<const DO_TWO_COMPL: bool>(val: &mut U256) -> Sign {
    if u256_high(*val) & SIGN_BITMASK_U128 == 0 {
        if *val == U256::zero() {
            Sign::Zero
        } else {
            Sign::Plus
        }
    } else {
        if DO_TWO_COMPL {
            two_compl_mut(val);
        }
        Sign::Minus
    }
}

#[inline(always)]
pub fn i256_div(mut first: U256, mut second: U256) -> U256 {
    let min_negative_value: U256 = u128_words_to_u256(SIGN_BITMASK_U128, 0);
    let second_sign = i256_sign::<true>(&mut second);
    if second_sign == Sign::Zero {
        return U256::zero();
    }
    let first_sign = i256_sign::<true>(&mut first);
    if first_sign == Sign::Minus && first == min_negative_value && second == U256::from(1) {
        return two_compl(min_negative_value);
    }

    let mut d = first / second;

    u256_remove_sign(&mut d);

    if d == U256::zero() {
        return U256::zero();
    }

    match (first_sign, second_sign) {
        (Sign::Zero, Sign::Plus)
        | (Sign::Plus, Sign::Zero)
        | (Sign::Zero, Sign::Zero)
        | (Sign::Plus, Sign::Plus)
        | (Sign::Minus, Sign::Minus) => d,
        (Sign::Zero, Sign::Minus)
        | (Sign::Plus, Sign::Minus)
        | (Sign::Minus, Sign::Zero)
        | (Sign::Minus, Sign::Plus) => two_compl(d),
    }
}

#[inline(always)]
pub fn i256_mod(mut first: U256, mut second: U256) -> U256 {
    let first_sign = i256_sign::<true>(&mut first);
    if first_sign == Sign::Zero {
        return U256::zero();
    }

    let _ = i256_sign::<true>(&mut second);
    let mut r = first % second;
    u256_remove_sign(&mut r);
    if r == U256::zero() {
        return U256::zero();
    }
    if first_sign == Sign::Minus {
        two_compl(r)
    } else {
        r
    }
}

#[inline(always)]
pub fn i256_cmp(mut first: U256, mut second: U256) -> Ordering {
    let first_sign = i256_sign::<false>(&mut first);
    let second_sign = i256_sign::<false>(&mut second);
    match (first_sign, second_sign) {
        (Sign::Zero, Sign::Zero) => Ordering::Equal,
        (Sign::Zero, Sign::Plus) => Ordering::Less,
        (Sign::Zero, Sign::Minus) => Ordering::Greater,
        (Sign::Minus, Sign::Zero) => Ordering::Less,
        (Sign::Minus, Sign::Plus) => Ordering::Less,
        (Sign::Minus, Sign::Minus) => first.cmp(&second),
        (Sign::Plus, Sign::Minus) => Ordering::Greater,
        (Sign::Plus, Sign::Zero) => Ordering::Greater,
        (Sign::Plus, Sign::Plus) => first.cmp(&second),
    }
}

#[inline(always)]
fn u256_remove_sign(val: &mut U256) {
    let low = u256_low(*val);
    let mut high = u256_high(*val);
    high &= FLIPH_BITMASK_U128;
    *val = u128_words_to_u256(high, low)
}

#[cfg(test)]
mod tests {
    use {super::*, core::num::Wrapping};

    #[test]
    fn div_i256() {
        let min_negative_value: U256 = u128_words_to_u256(SIGN_BITMASK_U128, 0);

        assert_eq!(Wrapping(i8::MIN) / Wrapping(-1), Wrapping(i8::MIN));
        assert_eq!(i8::MAX / -1, -i8::MAX);

        let one = U256::from(1);
        let one_hundred = U256::from(100);
        let fifty = U256::from(50);
        let _fifty_sign = Sign::Plus;
        let two = U256::from(2);
        let neg_one_hundred = U256::from(100);
        let _neg_one_hundred_sign = Sign::Minus;
        let minus_one = U256::from(1);
        let max_value = U256::from(2).pow(255.into()) - 1;
        let neg_max_value = U256::from(2).pow(255.into()) - 1;

        assert_eq!(i256_div(min_negative_value, minus_one), min_negative_value);
        assert_eq!(i256_div(min_negative_value, one), min_negative_value);
        assert_eq!(i256_div(max_value, one), max_value);
        assert_eq!(i256_div(max_value, minus_one), neg_max_value);
        assert_eq!(i256_div(one_hundred, minus_one), neg_one_hundred);
        assert_eq!(i256_div(one_hundred, two), fifty);
    }
}
