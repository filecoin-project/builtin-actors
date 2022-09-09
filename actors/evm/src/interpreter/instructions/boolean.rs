use {
    crate::interpreter::stack::Stack, crate::interpreter::uints, crate::interpreter::StatusCode,
    crate::interpreter::U256, std::cmp::Ordering,
};

#[inline]
pub fn lt(stack: &mut Stack) -> Result<(), StatusCode> {
    stack.apply::<2, _>(|args| {
        let a = args[1];
        let b = args[0];

        if a.lt(&b) {
            U256::from(1)
        } else {
            U256::zero()
        }
    })
}

#[inline]
pub fn gt(stack: &mut Stack) -> Result<(), StatusCode> {
    stack.apply::<2, _>(|args| {
        let a = args[1];
        let b = args[0];

        if a.gt(&b) {
            U256::from(1)
        } else {
            U256::zero()
        }
    })
}

#[inline]
pub fn slt(stack: &mut Stack) -> Result<(), StatusCode> {
    stack.apply::<2, _>(|args| {
        let a = args[1];
        let b = args[0];

        if uints::i256_cmp(a, b) == Ordering::Less {
            U256::from(1)
        } else {
            U256::zero()
        }
    })
}

#[inline]
pub fn sgt(stack: &mut Stack) -> Result<(), StatusCode> {
    stack.apply::<2, _>(|args| {
        let a = args[1];
        let b = args[0];

        if uints::i256_cmp(a, b) == Ordering::Greater {
            U256::from(1)
        } else {
            U256::zero()
        }
    })
}

#[inline]
pub fn eq(stack: &mut Stack) -> Result<(), StatusCode> {
    stack.apply::<2, _>(|args| {
        let a = args[1];
        let b = args[0];

        if a.eq(&b) {
            U256::from(1)
        } else {
            U256::zero()
        }
    })
}

#[inline]
pub fn iszero(stack: &mut Stack) -> Result<(), StatusCode> {
    stack.apply::<1, _>(|args| {
        let a = args[0];
        if a.is_zero() {
            U256::from(1)
        } else {
            U256::zero()
        }
    })
}

#[inline]
pub fn and(stack: &mut Stack) -> Result<(), StatusCode> {
    stack.apply::<2, _>(|args| {
        let a = args[1];
        let b = args[0];
        a & b
    })
}

#[inline]
pub fn or(stack: &mut Stack) -> Result<(), StatusCode> {
    stack.apply::<2, _>(|args| {
        let a = args[1];
        let b = args[0];
        a | b
    })
}

#[inline]
pub fn xor(stack: &mut Stack) -> Result<(), StatusCode> {
    stack.apply::<2, _>(|args| {
        let a = args[1];
        let b = args[0];
        a ^ b
    })
}

#[inline]
pub fn not(stack: &mut Stack) -> Result<(), StatusCode> {
    stack.apply::<1, _>(|args| {
        let v = args[0];
        !v
    })
}
