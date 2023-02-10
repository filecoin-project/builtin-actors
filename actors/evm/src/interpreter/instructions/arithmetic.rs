use fil_actors_evm_shared::uints::{U256, U512};

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
    a.i256_div(&b)
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
    a.i256_mod(&b)
}

#[inline]
pub fn addmod(a: U256, b: U256, c: U256) -> U256 {
    if !c.is_zero() {
        let al: U512 = a.into();
        let bl: U512 = b.into();
        let cl: U512 = c.into();

        ((al + bl) % cl).low_u256()
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
        ((al * bl) % cl).low_u256()
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
    mod basic {
        use super::super::*;
        use fil_actors_evm_shared::uints::U256;

        #[test]
        fn test_addmod() {
            assert_eq!(addmod(4.into(), 5.into(), 3.into()), 0, "4 + 5 % 3 = 0");
            assert_eq!(addmod(0.into(), 5.into(), 3.into()), 2, "0 + 5 % 3 = 2");
            assert_eq!(addmod(0.into(), 0.into(), 3.into()), 0, "0 + 0 % 3 = 0");
            // per evm, mod 0 is 0
            assert_eq!(addmod(1.into(), 2.into(), 0.into()), 0, "1 + 2 % 0 = 0");

            assert_eq!(addmod(U256::MAX, U256::MAX, 4.into()), 2, "max + max % 4 = 2");
        }

        #[test]
        fn test_mulmod() {
            assert_eq!(mulmod(4.into(), 5.into(), 3.into()), 2, "4 * 5 % 3 = 2");
            assert_eq!(mulmod(0.into(), 5.into(), 3.into()), 0, "0 * 5 % 3 = 0");
            assert_eq!(mulmod(0.into(), 0.into(), 3.into()), 0, "0 * 0 % 3 = 0");
            // per evm, mod 0 is 0
            assert_eq!(mulmod(1.into(), 2.into(), 0.into()), 0, "1 * 2 % 0 = 0");

            assert_eq!(mulmod(U256::MAX, U256::MAX, 2.into()), 1, "max * max % 2 = 1");
        }

        #[test]
        fn test_add() {
            assert_eq!(add(0.into(), 0.into()), 0, "add nothing to nothing");
            // does "math" on all limbs, so it is different than above
            assert_eq!(
                add(U256::max_value(), 0.into()),
                U256::max_value(),
                "add nothing to max value"
            );
            assert_eq!(add(2.into(), 2.into()), 4, "2 plus 2 equals 5 (???)");
            assert_eq!(
                add((u64::MAX).into(), 32.into()),
                U256::from(u64::MAX as u128 + 32),
                "add 32 past a single (u64) limb of u256"
            );
            // wrap to zero
            assert_eq!(add(U256::max_value(), 1.into()), 0, "overflow by one");
            // wrap all limbs
            assert_eq!(
                add(U256::max_value(), U256::max_value()),
                U256::max_value() - 1,
                "overflow by max, should be 2^256-1"
            );
        }

        #[test]
        fn test_mul() {
            assert_eq!(mul(0.into(), 0.into()), 0, "multiply nothing by nothing");
            assert_eq!(mul(2.into(), 3.into()), 6, "multiply 2 by 3");
            assert_eq!(
                mul(u64::MAX.into(), 2.into()),
                U256::from((u64::MAX as u128) * 2),
                "2^64 x 2"
            );
        }

        #[test]
        fn test_sub() {
            assert_eq!(sub(0.into(), 0.into()), 0, "subtract nothing by nothing");
            assert_eq!(sub(2.into(), 1.into()), 1, "subtract 2 by 1");
            assert_eq!(
                sub(((u64::MAX as u128) + 32).into(), 64.into()),
                u64::MAX - 32,
                "subtract 64 from a value 32 over a single limb"
            );
            // wrap to max
            assert_eq!(sub(0.into(), 1.into()), U256::max_value(), "wrap around to max by one");
            // wrap all limbs
            assert_eq!(sub(U256::max_value(), U256::max_value()), 0, "wrap around zero by 2^256");
        }

        #[test]
        fn test_div() {
            assert_eq!(div(0.into(), 0.into()), 0, "divide nothing by nothing (yes)");
            assert_eq!(div(4.into(), 1.into()), 4, "divide 4 by 1");
            assert_eq!(
                div((u128::MAX).into(), 2.into()),
                U256::from(u128::MAX / 2),
                "divide 2^128 by 2 (uses >1 limb)"
            );
        }

        #[test]
        fn test_modulo() {
            assert_eq!(modulo(0.into(), 0.into()), 0, "nothing mod nothing is nothing");
            assert_eq!(modulo(4.into(), 1.into()), 0, "4 mod 1 is 0");
            assert_eq!(
                modulo((u128::MAX).into(), 2.into()),
                U256::from(u128::MAX % 2),
                "2^128 mod 2"
            );

            assert_eq!(
                modulo((u128::MAX).into(), 7.into()),
                U256::from(u128::MAX % 7),
                "2^128 mod 7"
            );
        }

        #[test]
        fn test_signextend() {
            macro_rules! assert_exp {
                ($num:expr, $byte:expr, $result:expr) => {
                    let res: U256 = $result.into();
                    assert_eq!(res, signextend(($byte).into(), ($num).into()));
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
                    let res: U256 = $result.into();
                    assert_eq!(res, exp(($base).into(), ($exp).into()));
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
}
