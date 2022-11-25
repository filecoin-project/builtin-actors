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
            U256::ONE
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
    use crate::interpreter::stack::Stack;

    #[test]
    fn test_instruction_byte() {
        let value = U256::from_big_endian(&(1u8..=32u8).map(|x| 5 * x).collect::<Vec<u8>>());

        for i in 0u16..32 {
            let mut stack = Stack::new();
            unsafe {
                stack.push(value);
                stack.push(U256::from(i));
            }

            crate::interpreter::instructions::BYTE(&mut stack).unwrap();
            let result = unsafe { stack.pop() };

            assert_eq!(result, U256::from(5 * (i + 1)));
        }

        let mut stack = Stack::new();
        unsafe {
            stack.push(value);
            stack.push(U256::from(100u128));
        }

        crate::interpreter::instructions::BYTE(&mut stack).unwrap();
        let result = unsafe { stack.pop() };
        assert_eq!(result, U256::zero());

        let mut stack = Stack::new();
        unsafe {
            stack.push(value);
            stack.push(U256::from_u128_words(1, 0));
        }

        crate::interpreter::instructions::BYTE(&mut stack).unwrap();
        let result = unsafe { stack.pop() };
        assert_eq!(result, U256::zero());
    }
}
