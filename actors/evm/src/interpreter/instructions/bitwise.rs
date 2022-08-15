use {
    crate::interpreter::stack::Stack, crate::interpreter::uints::Sign,
    crate::interpreter::uints::*, crate::interpreter::U256,
};

#[inline]
pub fn byte(stack: &mut Stack) {
    let i = stack.pop();
    let x = stack.get_mut(0);

    if i >= U256::from(32) {
        *x = U256::zero();
        return;
    }

    let mut i = u256_low(i);

    let x_word = if i >= 16 {
        i -= 16;
        u256_low(*x)
    } else {
        u256_high(*x)
    };

    *x = U256::from((x_word >> (120 - i * 8)) & 0xFF);
}

#[inline]
pub fn shl(stack: &mut Stack) {
    let shift = stack.pop();
    let value = stack.get_mut(0);

    if *value == U256::zero() || shift >= U256::from(256) {
        *value = U256::zero();
    } else {
        *value <<= shift
    };
}

#[inline]
pub fn shr(stack: &mut Stack) {
    let shift = stack.pop();
    let value = stack.get_mut(0);

    if *value == U256::zero() || shift >= U256::from(256) {
        *value = U256::zero()
    } else {
        *value >>= shift
    };
}

#[inline]
pub fn sar(stack: &mut Stack) {
    let shift = stack.pop();
    let mut value = stack.pop();

    let value_sign = i256_sign::<true>(&mut value);

    stack.push(if value == U256::zero() || shift >= U256::from(256) {
        match value_sign {
            // value is 0 or >=1, pushing 0
            Sign::Plus | Sign::Zero => U256::zero(),
            // value is <0, pushing -1
            Sign::Minus => two_compl(U256::from(1)),
        }
    } else {
        let shift = shift.as_u128();

        match value_sign {
            Sign::Plus | Sign::Zero => value >> shift,
            Sign::Minus => {
                let shifted = ((value.overflowing_sub(U256::from(1)).0) >> shift)
                    .overflowing_add(U256::from(1))
                    .0;
                two_compl(shifted)
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use {super::*, crate::interpreter::uints::u128_words_to_u256};

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
        stack.push(u128_words_to_u256(1, 0));

        byte(&mut stack);
        let result = stack.pop();
        assert_eq!(result, U256::zero());
    }
}
