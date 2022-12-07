use {crate::interpreter::uints::*, crate::interpreter::U256, std::cmp::Ordering};

#[inline]
pub fn lt(a: U256, b: U256) -> U256 {
    U256::from_u64((a < b).into())
}

#[inline]
pub fn gt(a: U256, b: U256) -> U256 {
    U256::from_u64((a > b).into())
}

#[inline]
pub(crate) fn slt(a: U256, b: U256) -> U256 {
    U256::from_u64((i256_cmp(a, b) == Ordering::Less).into())
}

#[inline]
pub(crate) fn sgt(a: U256, b: U256) -> U256 {
    U256::from_u64((i256_cmp(a, b) == Ordering::Greater).into())
}

#[inline]
pub fn eq(a: U256, b: U256) -> U256 {
    U256::from_u64((a == b).into())
}

#[inline]
pub fn iszero(a: U256) -> U256 {
    U256::from_u64(a.is_zero().into())
}

#[inline]
pub(crate) fn and(a: U256, b: U256) -> U256 {
    a & b
}

#[inline]
pub(crate) fn or(a: U256, b: U256) -> U256 {
    a | b
}

#[inline]
pub(crate) fn xor(a: U256, b: U256) -> U256 {
    a ^ b
}

#[inline]
pub(crate) fn not(v: U256) -> U256 {
    !v
}
