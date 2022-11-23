//! ## Implementer's notes
//!
//! All operations are done with overflowing math
//! TODO (simple?) add, mul, sub div
//!
//! ### Non-critical TODOs
//! Many operations can simply mutate the last value on stack instead of pop/push.

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
    // TODO shortcut optimizations from go's lib (our doesn't)
    // https://github.com/holiman/uint256/blob/6f8ccba90ce6cba9727ad5aa26bb925a25b50d29/uint256.go#L544
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

    if a < 32 {
        let bit_index = 8 * a.low_u32() + 7;
        let mask = U256::MAX >> (U256::BITS - bit_index);
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

    mod basic {
        use crate::interpreter::{stack::Stack, U256};

        // all operations go a <op> b
        fn push_2(s: &mut Stack, a: impl Into<U256>, b: impl Into<U256>) {
            s.push(b.into());
            s.push(a.into());
        }

        fn expect_stack_value(s: &mut Stack, e: impl Into<U256>, comment: impl AsRef<str>) {
            let mut expected = Stack::new();
            expected.push(e.into());

            // stacks should be _exactly_ the same
            assert_eq!(s.0, expected.0, "{}", comment.as_ref());
            s.pop();
        }

        #[test]
        fn add() {
            let mut s = Stack::default();
            let s = &mut s;

            push_2(s, 0, 0);
            super::add(s);
            expect_stack_value(s, 0, "add nothing to nothing");

            // does "math" on all limbs, so it is different than above
            push_2(s, U256::max_value(), 0);
            super::add(s);
            expect_stack_value(s, U256::max_value(), "add nothing to max value");

            push_2(s, 2, 2);
            super::add(s);
            expect_stack_value(s, 4, "2 plus 2 equals 5 (???)");

            push_2(s, u64::MAX, 32);
            super::add(s);
            expect_stack_value(s, u64::MAX as u128 + 32, "add 32 past a single (u64) limb of u256");

            // wrap to zero
            push_2(s, U256::max_value(), 1);
            super::add(s);
            expect_stack_value(s, 0, "overflow by one");

            // wrap all limbs
            push_2(s, U256::max_value(), U256::max_value());
            super::add(s);
            expect_stack_value(s, U256::max_value() - 1, "overflow by max, should be 2^256-1");
        }

        #[test]
        fn mul() {
            let mut s = Stack::default();
            let s = &mut s;

            push_2(s, 0, 0);
            super::mul(s);
            expect_stack_value(s, 0, "multiply nothing by nothing");

            push_2(s, 2, 3);
            super::mul(s);
            expect_stack_value(s, 6, "multiply 2 by 3");

            push_2(s, u64::MAX, 2);
            super::mul(s);
            expect_stack_value(s, (u64::MAX as u128) * 2, "2^64 x 2");
        }

        #[test]
        fn sub() {
            let mut s = Stack::default();
            let s = &mut s;

            push_2(s, 0, 0);
            super::sub(s);
            expect_stack_value(s, 0, "subtract nothing by nothing");

            push_2(s, 2, 1);
            super::sub(s);
            expect_stack_value(s, 1, "subtract 2 by 1");

            push_2(s, (u64::MAX as u128) + 32, 64);
            super::sub(s);
            expect_stack_value(s, u64::MAX - 32, "subtract 64 from a value 32 over a single limb");

            // wrap to max
            push_2(s, 0, 1);
            super::sub(s);
            expect_stack_value(s, U256::max_value(), "wrap around to max by one");

            // wrap all limbs
            push_2(s, U256::max_value(), U256::max_value());
            super::sub(s);
            expect_stack_value(s, 0, "wrap around zero by 2^256");
        }

        #[test]
        fn div() {
            let mut s = Stack::default();
            let s = &mut s;

            push_2(s, 0, 0);
            super::div(s);
            expect_stack_value(s, 0, "divide nothing by nothing (yes)");

            push_2(s, 4, 1);
            super::div(s);
            expect_stack_value(s, 4, "divide 4 by 1");

            push_2(s, u128::MAX, 2);
            super::div(s);
            expect_stack_value(s, u128::MAX / 2, "divide 2^128 by 2 (uses >1 limb)");
        }
    }

    use super::*;
    #[test]
    fn test_signextend() {
        macro_rules! assert_exp {
            ($num:expr, $byte:expr, $result:expr) => {
                let mut stack = Stack::new();
                stack.push(($num).into());
                stack.push(($byte).into());
                signextend(&mut stack);
                let res: U256 = ($result).into();
                assert_eq!(res, stack.pop());
            };
        }
        assert_exp!(0xff, 0, U256::MAX);
        assert_exp!(0xff, 1, 0xff);
        assert_exp!(0xf0, 0, !U256::from_u64(0x0f));
        // Large
        assert_exp!(
            U256::from_u128_words(0x82, 0x1),
            16,
            U256::from_u128_words((u128::MAX ^ 0xff) | 0x82, 0x1)
        );
        assert_exp!(U256::from_u128_words(0x82, 0x1), 15, U256::from_u128_words(0x0, 0x1));
        assert_exp!(U256::from_u128_words(0x82, 0x1), 17, U256::from_u128_words(0x82, 0x1));
        // Not At Boundary
        assert_exp!(U256::from_u128_words(0x62, 0x1), 16, U256::from_u128_words(0x62, 0x1));
    }
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
