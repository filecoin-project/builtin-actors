use {
    crate::interpreter::stack::Stack, crate::interpreter::uints::*, crate::interpreter::U256,
    std::cmp::Ordering,
};

#[inline]
pub fn lt(stack: &mut Stack) {
    let a = stack.pop();
    let b = stack.get_mut(0);

    *b = U256::from_u64((a < *b).into());
}

#[inline]
pub fn gt(stack: &mut Stack) {
    let a = stack.pop();
    let b = stack.get_mut(0);

    *b = U256::from_u64((a > *b).into());
}

#[inline]
pub(crate) fn slt(stack: &mut Stack) {
    let a = stack.pop();
    let b = stack.get_mut(0);

    *b = U256::from_u64((i256_cmp(a, *b) == Ordering::Less).into());
}

#[inline]
pub(crate) fn sgt(stack: &mut Stack) {
    let a = stack.pop();
    let b = stack.get_mut(0);

    *b = U256::from_u64((i256_cmp(a, *b) == Ordering::Greater).into());
}

#[inline]
pub fn eq(stack: &mut Stack) {
    let a = stack.pop();
    let b = stack.get_mut(0);

    *b = U256::from_u64((a == *b).into());
}

#[inline]
pub fn iszero(stack: &mut Stack) {
    let a = stack.get_mut(0);
    *a = U256::from_u64(a.is_zero().into());
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
