use {
    crate::interpreter::StatusCode,
    crate::interpreter::U256,
    crate::interpreter::stack::Stack,
    crate::interpreter::uints,
};

#[inline]
pub fn byte(stack: &mut Stack) -> Result<(), StatusCode> {
    stack.apply::<2,_>(|args| {
        let i = args[1];
        let x = args[0];

        if i >= U256::from(32) {
            return U256::zero();
        }

        let mut i = uints::u256_low(i);

        let x_word = if i >= 16 {
            i -= 16;
            uints::u256_low(x)
        } else {
            uints::u256_high(x)
        };

        U256::from((x_word >> (120 - i * 8)) & 0xFF)
    })
}

#[inline]
pub fn shl(stack: &mut Stack) -> Result<(), StatusCode> {
    stack.apply::<2,_>(|args| {
        let shift = args[1];
        let value = args[0];

        if value.is_zero() || shift >= U256::from(256) {
            U256::zero()
        } else {
            value << shift
        }
    })
}

#[inline]
pub fn shr(stack: &mut Stack) -> Result<(), StatusCode> {
    stack.apply::<2,_>(|args| {
        let shift = args[1];
        let value = args[0];

        if value.is_zero() || shift >= U256::from(256) {
            U256::zero()
        } else {
            value >> shift
        }
    })
}

#[inline]
pub fn sar(stack: &mut Stack) -> Result<(), StatusCode> {
    stack.apply::<2,_>(|args| {
        let shift = args[1];
        let mut value = args[0];

        let value_sign = uints::i256_sign::<true>(&mut value);

        if value.is_zero() || shift >= U256::from(256) {
            match value_sign {
                // value is 0 or >=1, pushing 0
                uints::Sign::Plus | uints::Sign::Zero => U256::zero(),
                // value is <0, pushing -1
                uints::Sign::Minus => uints::two_compl(U256::from(1)),
            }
        } else {
            let shift = shift.as_u128();

            match value_sign {
                uints::Sign::Plus | uints::Sign::Zero => value >> shift,
                uints::Sign::Minus => {
                    let shifted = ((value.overflowing_sub(U256::from(1)).0) >> shift)
                        .overflowing_add(U256::from(1))
                        .0;
                    uints::two_compl(shifted)
                }
            }
        }
    })
}

#[cfg(test)]
mod tests {
    use {super::*, crate::interpreter::uints::u128_words_to_u256};

    #[test]
    fn test_instruction_byte() {
        let value = U256::from_big_endian(&(1u8..=32u8).map(|x| 5 * x).collect::<Vec<u8>>());

        for i in 0u16..32 {
            let mut stack = Stack::new();
            stack.push(value).unwrap();
            stack.push(U256::from(i)).unwrap();

            byte(&mut stack).unwrap();
            let result = stack.pop().unwrap();

            assert_eq!(result, U256::from(5 * (i + 1)));
        }

        let mut stack = Stack::new();
        stack.push(value).unwrap();
        stack.push(U256::from(100u128)).unwrap();

        byte(&mut stack).unwrap();
        let result = stack.pop().unwrap();
        assert_eq!(result, U256::zero());

        let mut stack = Stack::new();
        stack.push(value).unwrap();
        stack.push(u128_words_to_u256(1, 0)).unwrap();

        byte(&mut stack).unwrap();
        let result = stack.pop().unwrap();
        assert_eq!(result, U256::zero());
    }
}
