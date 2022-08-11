use {
    crate::interpreter::stack::Stack, crate::interpreter::uints::*, crate::interpreter::U256, std::cmp::Ordering,
};

#[inline]
pub fn lt(stack: &mut Stack) {
    let a = stack.pop();
    let b = stack.get_mut(0);

    *b = if a.lt(b) { U256::from(1) } else { U256::zero() }
}

#[inline]
pub fn gt(stack: &mut Stack) {
    let a = stack.pop();
    let b = stack.get_mut(0);

    *b = if a.gt(b) { U256::from(1) } else { U256::zero() }
}

#[inline]
pub(crate) fn slt(stack: &mut Stack) {
    let a = stack.pop();
    let b = stack.get_mut(0);

    *b = if i256_cmp(a, *b) == Ordering::Less { U256::from(1) } else { U256::zero() }
}

#[inline]
pub(crate) fn sgt(stack: &mut Stack) {
    let a = stack.pop();
    let b = stack.get_mut(0);

    *b = if i256_cmp(a, *b) == Ordering::Greater { U256::from(1) } else { U256::zero() }
}

#[inline]
pub fn eq(stack: &mut Stack) {
    let a = stack.pop();
    let b = stack.get_mut(0);

    *b = if a.eq(b) { U256::from(1) } else { U256::zero() }
}

#[inline]
pub fn iszero(stack: &mut Stack) {
    let a = stack.get_mut(0);
    *a = if *a == U256::zero() { U256::from(1) } else { U256::zero() }
}

#[inline]
pub(crate) fn and(stack: &mut Stack) {
    let a = stack.pop();
    let b = stack.get_mut(0);
    *b = a & *b;
}

#[inline]
pub(crate) fn or(stack: &mut Stack) {
    let a = stack.pop();
    let b = stack.get_mut(0);
    *b = a | *b;
}

#[inline]
pub(crate) fn xor(stack: &mut Stack) {
    let a = stack.pop();
    let b = stack.get_mut(0);
    *b = a ^ *b;
}

#[inline]
pub(crate) fn not(stack: &mut Stack) {
    let v = stack.get_mut(0);
    *v = !*v;
}
