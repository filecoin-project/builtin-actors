use {crate::interpreter::uints::*, crate::interpreter::U256};

#[inline]
pub fn add(a: U256, b: U256) -> U256 {
    a.overflowing_add(b).0
}

#[inline]
pub fn mul(a: U256, b: U256) -> U256 {
    a.overflowing_mul(b).0
}

#[inline]
pub fn sub(a: U256, b: U256) -> U256 {
    a.overflowing_sub(b).0
}

#[inline]
pub fn div(a: U256, b: U256) -> U256 {
    // TODO shortcut optimizations from go's lib (our doesn't)
    // https://github.com/holiman/uint256/blob/6f8ccba90ce6cba9727ad5aa26bb925a25b50d29/uint256.go#L544
    if !b.is_zero() {
        a / b
    } else {
        b
    }
}

#[inline]
pub fn sdiv(a: U256, b: U256) -> U256 {
    i256_div(a, b)
}

#[inline]
pub fn modulo(a: U256, b: U256) -> U256 {
    if !b.is_zero() {
        a % b
    } else {
        b
    }
}

#[inline]
pub fn smod(a: U256, b: U256) -> U256 {
    i256_mod(a, b)
}

#[inline]
pub fn addmod(a: U256, b: U256, c: U256) -> U256 {
    if !c.is_zero() {
        let al: U512 = a.into();
        let bl: U512 = b.into();
        let cl: U512 = c.into();

        (al + bl % cl).low_u256()
    } else {
        c
    }
}

#[inline]
pub fn mulmod(a: U256, b: U256, c: U256) -> U256 {
    if !c.is_zero() {
        let al: U512 = a.into();
        let bl: U512 = b.into();
        let cl: U512 = c.into();
        (al * bl % cl).low_u256()
    } else {
        c
    }
}

#[inline]
pub fn signextend(a: U256, b: U256) -> U256 {
    if a < 32 {
        let bit_index = 8 * a.low_u32() + 7;
        let mask = U256::MAX >> (U256::BITS - bit_index);
        if b.bit(bit_index as usize) {
            b | !mask
        } else {
            b & mask
        }
    } else {
        b
    }
}

#[inline]
pub fn exp(mut base: U256, power: U256) -> U256 {
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

    v
}

#[cfg(test)]
mod test {
    use super::*;
    use crate::interpreter::stack::Stack;

    mod basic {
        use crate::interpreter::{stack::Stack, U256};

        // all operations go a <op> b
        fn push_2(s: &mut Stack, a: impl Into<U256>, b: impl Into<U256>) {
            unsafe {
                s.push(b.into());
                s.push(a.into());
            }
        }

        fn expect_stack_value(s: &mut Stack, e: impl Into<U256>, comment: impl AsRef<str>) {
            let mut expected = Stack::new();
            unsafe {
                expected.push(e.into());
            }

            // stacks should be _exactly_ the same
            assert_eq!(unsafe { s.get(0) }, unsafe { expected.get(0) }, "{}", comment.as_ref());
            unsafe {
                s.pop();
            }
        }

        #[test]
        fn add() {
            let mut s = Stack::default();
            let s = &mut s;

            push_2(s, 0, 0);
            crate::interpreter::instructions::ADD(s).unwrap();
            expect_stack_value(s, 0, "add nothing to nothing");

            // does "math" on all limbs, so it is different than above
            push_2(s, U256::max_value(), 0);
            crate::interpreter::instructions::ADD(s).unwrap();
            expect_stack_value(s, U256::max_value(), "add nothing to max value");

            push_2(s, 2, 2);
            crate::interpreter::instructions::ADD(s).unwrap();
            expect_stack_value(s, 4, "2 plus 2 equals 5 (???)");

            push_2(s, u64::MAX, 32);
            crate::interpreter::instructions::ADD(s).unwrap();
            expect_stack_value(s, u64::MAX as u128 + 32, "add 32 past a single (u64) limb of u256");

            // wrap to zero
            push_2(s, U256::max_value(), 1);
            crate::interpreter::instructions::ADD(s).unwrap();
            expect_stack_value(s, 0, "overflow by one");

            // wrap all limbs
            push_2(s, U256::max_value(), U256::max_value());
            crate::interpreter::instructions::ADD(s).unwrap();
            expect_stack_value(s, U256::max_value() - 1, "overflow by max, should be 2^256-1");
        }

        #[test]
        fn mul() {
            let mut s = Stack::default();
            let s = &mut s;

            push_2(s, 0, 0);
            crate::interpreter::instructions::MUL(s).unwrap();
            expect_stack_value(s, 0, "multiply nothing by nothing");

            push_2(s, 2, 3);
            crate::interpreter::instructions::MUL(s).unwrap();
            expect_stack_value(s, 6, "multiply 2 by 3");

            push_2(s, u64::MAX, 2);
            crate::interpreter::instructions::MUL(s).unwrap();
            expect_stack_value(s, (u64::MAX as u128) * 2, "2^64 x 2");
        }

        #[test]
        fn sub() {
            let mut s = Stack::default();
            let s = &mut s;

            push_2(s, 0, 0);
            crate::interpreter::instructions::SUB(s).unwrap();
            expect_stack_value(s, 0, "subtract nothing by nothing");

            push_2(s, 2, 1);
            crate::interpreter::instructions::SUB(s).unwrap();
            expect_stack_value(s, 1, "subtract 2 by 1");

            push_2(s, (u64::MAX as u128) + 32, 64);
            crate::interpreter::instructions::SUB(s).unwrap();
            expect_stack_value(s, u64::MAX - 32, "subtract 64 from a value 32 over a single limb");

            // wrap to max
            push_2(s, 0, 1);
            crate::interpreter::instructions::SUB(s).unwrap();
            expect_stack_value(s, U256::max_value(), "wrap around to max by one");

            // wrap all limbs
            push_2(s, U256::max_value(), U256::max_value());
            crate::interpreter::instructions::SUB(s).unwrap();
            expect_stack_value(s, 0, "wrap around zero by 2^256");
        }

        #[test]
        fn div() {
            let mut s = Stack::default();
            let s = &mut s;

            push_2(s, 0, 0);
            crate::interpreter::instructions::DIV(s).unwrap();
            expect_stack_value(s, 0, "divide nothing by nothing (yes)");

            push_2(s, 4, 1);
            crate::interpreter::instructions::DIV(s).unwrap();
            expect_stack_value(s, 4, "divide 4 by 1");

            push_2(s, u128::MAX, 2);
            crate::interpreter::instructions::DIV(s).unwrap();
            expect_stack_value(s, u128::MAX / 2, "divide 2^128 by 2 (uses >1 limb)");
        }
    }

    #[test]
    fn test_signextend() {
        macro_rules! assert_exp {
            ($num:expr, $byte:expr, $result:expr) => {
                let mut stack = Stack::new();
                unsafe {
                    stack.push(($num).into());
                    stack.push(($byte).into());
                }
                crate::interpreter::instructions::SIGNEXTEND(&mut stack).unwrap();
                let res: U256 = ($result).into();
                assert_eq!(res, unsafe { stack.pop() });
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
                unsafe {
                    stack.push(($exp).into());
                    stack.push(($base).into());
                }
                crate::interpreter::instructions::EXP(&mut stack).unwrap();
                let res: U256 = ($result).into();
                assert_eq!(res, unsafe { stack.pop() });
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
