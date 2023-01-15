use crate::interpreter::U256;

#[inline]
pub fn byte(i: U256, x: U256) -> U256 {
    if i >= 32 {
        U256::ZERO
    } else {
        U256::from_u64(x.byte(31 - i.low_u64() as usize) as u64)
    }
}

#[inline]
pub fn shl(shift: U256, value: U256) -> U256 {
    if value.is_zero() || shift >= 256 {
        U256::ZERO
    } else {
        value << shift
    }
}

#[inline]
pub fn shr(shift: U256, value: U256) -> U256 {
    if value.is_zero() || shift >= 256 {
        U256::ZERO
    } else {
        value >> shift
    }
}

#[inline]
pub fn sar(shift: U256, mut value: U256) -> U256 {
    let negative = value.i256_is_negative();
    if negative {
        value = value.i256_neg();
    }

    if value.is_zero() || shift >= 256 {
        if negative {
            // value is < 0, pushing U256::MAX (== -1)
            U256::MAX
        } else {
            // value is >= 0, pushing 0
            U256::ZERO
        }
    } else {
        let shift = shift.low_u32();

        if negative {
            let shifted =
                (value.overflowing_sub(U256::ONE).0 >> shift).overflowing_add(U256::ONE).0;
            shifted.i256_neg()
        } else {
            value >> shift
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_shl() {
        // Basic shift
        assert_eq!(shl(U256::from(2), U256::from(13)), U256::from(52));

        // 0/1 shifts.
        assert_eq!(shl(U256::ONE, U256::ONE), U256::from(2));
        assert_eq!(shl(U256::ONE, U256::ZERO), U256::ZERO);
        assert_eq!(shl(U256::ZERO, U256::ONE), U256::ONE);
        assert_eq!(shl(U256::ZERO, U256::ZERO), U256::ZERO);

        // shift max bits
        assert_eq!(shl(U256::ONE, U256::MAX), U256::MAX - U256::ONE);
        assert_eq!(shl(U256::from(2), U256::MAX), U256::MAX - U256::from(3));

        // shift by max
        assert_eq!(shl(U256::from(255), U256::MAX), U256::from_u128_words(i128::MIN as u128, 0));
        assert_eq!(shl(U256::from(256), U256::MAX), U256::ZERO);
        assert_eq!(shl(U256::from(257), U256::MAX), U256::ZERO);
    }

    #[test]
    fn test_shr() {
        // Basic shift
        assert_eq!(shr(U256::from(2), U256::from(13)), U256::from(3));

        // 0/1 shifts.
        assert_eq!(shr(U256::ONE, U256::ONE), U256::ZERO);
        assert_eq!(shr(U256::ONE, U256::ZERO), U256::ZERO);
        assert_eq!(shr(U256::ZERO, U256::ONE), U256::ONE);
        assert_eq!(shr(U256::ZERO, U256::ZERO), U256::ZERO);

        // shift max
        assert_eq!(shr(U256::from(255), U256::MAX), U256::ONE);
        assert_eq!(shr(U256::from(256), U256::MAX), U256::ZERO);
        assert_eq!(shr(U256::from(257), U256::MAX), U256::ZERO);
    }

    #[test]
    fn test_sar() {
        let pos_max = shr(U256::ONE, U256::MAX);

        // Basic shift
        assert_eq!(sar(U256::from(2), U256::from(13)), U256::from(3));
        assert_eq!(sar(U256::from(2), U256::from(13).i256_neg()), U256::from(4).i256_neg());

        // 0/1 shifts.
        assert_eq!(sar(U256::ONE, U256::ONE), U256::ZERO);
        assert_eq!(sar(U256::ONE, U256::ZERO), U256::ZERO);
        assert_eq!(sar(U256::ZERO, U256::ONE), U256::ONE);
        assert_eq!(sar(U256::ZERO, U256::ZERO), U256::ZERO);

        // shift max negative
        assert_eq!(sar(U256::from(255), U256::MAX), U256::MAX); // sign extends.
        assert_eq!(sar(U256::from(256), U256::MAX), U256::MAX);
        assert_eq!(sar(U256::from(257), U256::MAX), U256::MAX);

        // shift max positive.
        assert_eq!(sar(U256::from(254), pos_max), U256::ONE);
        assert_eq!(sar(U256::from(255), pos_max), U256::ZERO);
        assert_eq!(sar(U256::from(256), pos_max), U256::ZERO);
        assert_eq!(sar(U256::from(257), pos_max), U256::ZERO);
    }

    #[test]
    fn test_instruction_byte() {
        let value = U256::from_big_endian(&(1u8..=32u8).map(|x| 5 * x).collect::<Vec<u8>>());

        for i in 0u16..32 {
            let result = byte(U256::from(i), value);

            assert_eq!(result, U256::from(5 * (i + 1)));
        }

        let result = byte(U256::from(100u128), value);
        assert_eq!(result, U256::zero());

        let result = byte(U256::from_u128_words(1, 0), value);
        assert_eq!(result, U256::zero());
    }
}
