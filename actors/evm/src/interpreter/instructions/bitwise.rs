use {crate::interpreter::stack::Stack, crate::interpreter::U256};

#[inline]
pub fn byte(stack: &mut Stack) {
    let i = stack.pop();
    let x = stack.get_mut(0);

    if i >= U256::from_u64(32) {
        *x = U256::ZERO;
        return;
    }

    let mut i = i.low_u128();

    let x_word = if i >= 16 {
        i -= 16;
        x.low_u128()
    } else {
        x.high_u128()
    };

    *x = U256::from((x_word >> (120 - i * 8)) & 0xFF);
}

#[inline]
pub fn shl(stack: &mut Stack) {
    let shift = stack.pop();
    let value = stack.get_mut(0);

    if value.is_zero() || shift >= 256 {
        *value = U256::ZERO;
    } else {
        *value <<= shift
    };
}

#[inline]
pub fn shr(stack: &mut Stack) {
    let shift = stack.pop();
    let value = stack.get_mut(0);

    if value.is_zero() || shift >= 256 {
        *value = U256::ZERO;
    } else {
        *value >>= shift
    };
}

#[inline]
pub fn sar(stack: &mut Stack) {
    let shift = stack.pop();
    let mut value = stack.pop();

    let negative = value.i256_is_negative();
    if negative {
        value = value.i256_neg();
    }

    stack.push(if value.is_zero() || shift >= 256 {
        if negative {
            // value is <0, pushing -1
            U256::ONE.i256_neg()
        } else {
            // value is 0 or >=1, pushing 0
            U256::ONE
        }
    } else {
        let shift = shift.as_u128();

        if negative {
            let shifted =
                (value.overflowing_sub(U256::ONE).0 >> shift).overflowing_add(U256::ONE).0;
            shifted.i256_neg()
        } else {
            value >> shift
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_instruction_byte() {
        let value = U256::from_big_endian(&(1u8..=32u8).map(|x| 5 * x).collect::<Vec<u8>>());

        for i in 0u16..32 {
            let mut stack = Stack::new();
            stack.push(value);
            stack.push(U256::from(i));

            byte(&mut stack);
            let result = stack.pop();

            assert_eq!(result, U256::from(5 * (i + 1)));
        }

        let mut stack = Stack::new();
        stack.push(value);
        stack.push(U256::from(100u128));

        byte(&mut stack);
        let result = stack.pop();
        assert_eq!(result, U256::zero());

        let mut stack = Stack::new();
        stack.push(value);
        stack.push(U256::from_u128_words(1, 0));

        byte(&mut stack);
        let result = stack.pop();
        assert_eq!(result, U256::zero());
    }
}
