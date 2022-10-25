use {crate::interpreter::stack::Stack, crate::interpreter::uints::*, crate::interpreter::U256};

#[inline]
pub fn add(stack: &mut Stack) {
    let a = stack.pop();
    let b = stack.pop();
    stack.push(a.overflowing_add(b).0);
}

#[inline]
pub fn mul(stack: &mut Stack) {
    let a = stack.pop();
    let b = stack.pop();
    stack.push(a.overflowing_mul(b).0);
}

#[inline]
pub fn sub(stack: &mut Stack) {
    let a = stack.pop();
    let b = stack.pop();
    stack.push(a.overflowing_sub(b).0);
}

#[inline]
pub fn div(stack: &mut Stack) {
    let a = stack.pop();
    let b = stack.get_mut(0);
    if !b.is_zero() {
        *b = a / *b
    }
}

#[inline]
pub fn sdiv(stack: &mut Stack) {
    let a = stack.pop();
    let b = stack.pop();
    let v = i256_div(a, b);
    stack.push(v);
}

#[inline]
pub fn modulo(stack: &mut Stack) {
    let a = stack.pop();
    let b = stack.get_mut(0);
    if !b.is_zero() {
        *b = a % *b;
    }
}

#[inline]
pub fn smod(stack: &mut Stack) {
    let a = stack.pop();
    let b = stack.get_mut(0);

    *b = i256_mod(a, *b);
}

#[inline]
pub fn addmod(stack: &mut Stack) {
    let a = stack.pop();
    let b = stack.pop();
    let c = stack.get_mut(0);

    if !c.is_zero() {
        let al: U512 = a.into();
        let bl: U512 = b.into();
        let cl: U512 = (*c).into();

        *c = (al + bl % cl).low_u256();
    }
}

#[inline]
pub fn mulmod(stack: &mut Stack) {
    let a = stack.pop();
    let b = stack.pop();
    let c = stack.get_mut(0);

    if !c.is_zero() {
        let al: U512 = a.into();
        let bl: U512 = b.into();
        let cl: U512 = (*c).into();
        *c = (al * bl % cl).low_u256();
    }
}

#[inline]
pub fn signextend(stack: &mut Stack) {
    let a = stack.pop();
    let b = stack.get_mut(0);

    if a < U256::from_u64(32) {
        let bit_index = 8 * a.low_u64() + 7;
        let mask = (U256::ONE << bit_index) - U256::ONE;
        *b = if b.bit(bit_index as usize) { *b | !mask } else { *b & mask }
    }
}

#[inline]
pub fn exp(stack: &mut Stack) {
    let mut base = stack.pop();
    let power = stack.pop();

    let mut v = U256::ONE;

    // First, compute the number of remaining significant bits.
    let mut remaining_bits = U256::BITS - power.leading_zeros();

    // Word by word, least significant to most.
    for mut word in power.0 {
        // While we have bits left...
        for _ in 0..u64::BITS.min(remaining_bits) {
            if (word & 1) != 0 {
                v = v.overflowing_mul(base).0;
            }
            word >>= 1;
            base = base.overflowing_mul(base).0;
        }
        remaining_bits = remaining_bits.saturating_sub(u64::BITS);
    }

    stack.push(v);
}

#[cfg(test)]
mod test {
    use super::*;
    #[test]
    fn test_exp() {
        macro_rules! assert_exp {
            ($base:expr, $exp:expr, $result:expr) => {
                let mut stack = Stack::new();
                stack.push(($exp).into());
                stack.push(($base).into());
                exp(&mut stack);
                let res: U256 = ($result).into();
                assert_eq!(res, stack.pop());
            };
        }

        // Basic tests.
        for (base, exp) in
            [(0u64, 0u32), (0, 1), (1, 0), (1, 10), (10, 1), (10, 0), (0, 10), (10, 10)]
        {
            assert_exp!(base, exp, base.pow(exp));
        }

        // BIG no-op tests
        assert_exp!(U256::from_u128_words(1, 0), 1, U256::from_u128_words(1, 0));
        assert_exp!(U256::from_u128_words(1, 0), 0, 1);

        // BIG actual tests
        assert_exp!(
            U256::from_u128_words(0, 1 << 65),
            2,
            U256::from_u128_words(4 /* 65 * 2 = 128 + 4 */, 0)
        );

        // Check overflow.
        assert_exp!(100, U256::from_u128_words(1, 0), U256::ZERO);
        assert_exp!(U256::from_u128_words(1, 0), 100, U256::ZERO);
        // Check big wrapping.
        assert_exp!(
            123,
            U256::from_u128_words(0, 123 << 64),
            U256::from_u128_words(
                0x9c4a2f94642e820e0f1d7d4208d629d8,
                0xd9ae51d86b0ede140000000000000001
            )
        );
        assert_exp!(
            U256::from_u128_words(0, 1 << 66) - U256::ONE,
            U256::from_u128_words(0, 1 << 67) - U256::ONE,
            U256::from_u128_words(0x80000000000000000f, 0xfffffffffffffffbffffffffffffffff)
        );
    }
}
