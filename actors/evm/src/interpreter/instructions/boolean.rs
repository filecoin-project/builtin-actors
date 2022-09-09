use {
    crate::interpreter::stack::Stack, crate::interpreter::uints, crate::interpreter::StatusCode,
    crate::interpreter::U256, std::cmp::Ordering,
};

#[inline]
pub fn lt(stack: &mut Stack) -> Result<(), StatusCode> {
    stack.apply2(|a, b| {
        if a.lt(&b) {
            U256::from(1)
        } else {
            U256::zero()
        }
    })
}

#[inline]
pub fn gt(stack: &mut Stack) -> Result<(), StatusCode> {
    stack.apply2(|a, b| {
        if a.gt(&b) {
            U256::from(1)
        } else {
            U256::zero()
        }
    })
}

#[inline]
pub fn slt(stack: &mut Stack) -> Result<(), StatusCode> {
    stack.apply2(|a, b| {
        if uints::i256_cmp(a, b) == Ordering::Less {
            U256::from(1)
        } else {
            U256::zero()
        }
    })
}

#[inline]
pub fn sgt(stack: &mut Stack) -> Result<(), StatusCode> {
    stack.apply2(|a, b| {
        if uints::i256_cmp(a, b) == Ordering::Greater {
            U256::from(1)
        } else {
            U256::zero()
        }
    })
}

#[inline]
pub fn eq(stack: &mut Stack) -> Result<(), StatusCode> {
    stack.apply2(|a, b| {
        if a.eq(&b) {
            U256::from(1)
        } else {
            U256::zero()
        }
    })
}

#[inline]
pub fn iszero(stack: &mut Stack) -> Result<(), StatusCode> {
    stack.apply1(|a| {
        if a.is_zero() {
            U256::from(1)
        } else {
            U256::zero()
        }
    })
}

#[inline]
pub fn and(stack: &mut Stack) -> Result<(), StatusCode> {
    stack.apply2(|a, b| {
        a & b
    })
}

#[inline]
pub fn or(stack: &mut Stack) -> Result<(), StatusCode> {
    stack.apply2(|a, b| {
        a | b
    })
}

#[inline]
pub fn xor(stack: &mut Stack) -> Result<(), StatusCode> {
    stack.apply2(|a, b| {
        a ^ b
    })
}

#[inline]
pub fn not(stack: &mut Stack) -> Result<(), StatusCode> {
    stack.apply1(|v| {
        !v
    })
}
