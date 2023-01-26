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

#[cfg(test)]
mod test {
    use super::*;


    #[test]
    fn test_and() {
        for i in 0..u8::MAX {
            for j in 0..u8::MAX {
                let a = U256::from(i);
                let b = U256::from(j);
                assert_eq!(and(a, b), U256::from(i & j))
            }
        } 

        assert_eq!(and(U256::MAX, U256::ZERO), U256::ZERO);
        assert_eq!(and(U256::ZERO, U256::MAX), U256::ZERO);
        assert_eq!(and(U256::MAX, U256::MAX), U256::MAX);
        assert_eq!(and(U256::ZERO, U256::ZERO), U256::ZERO);
    }

    #[test]
    fn test_or() {
        for i in 0..u8::MAX {
            for j in 0..u8::MAX {
                let a = U256::from(i);
                let b = U256::from(j);
                assert_eq!(or(a, b), U256::from(i | j))
            }
        } 

        assert_eq!(or(U256::MAX, U256::ZERO), U256::MAX);
        assert_eq!(or(U256::ZERO, U256::MAX), U256::MAX);
        assert_eq!(or(U256::MAX, U256::MAX), U256::MAX);
        assert_eq!(or(U256::ZERO, U256::ZERO), U256::ZERO);
    }

    #[test]
    fn test_xor() {
        for i in 0..u8::MAX {
            for j in 0..u8::MAX {
                let a = U256::from(i);
                let b = U256::from(j);
                assert_eq!(xor(a, b), U256::from(i ^ j))
            }
        } 

        assert_eq!(xor(U256::MAX, U256::ZERO), U256::MAX);
        assert_eq!(xor(U256::ZERO, U256::MAX), U256::MAX);
        assert_eq!(xor(U256::MAX, U256::MAX), U256::ZERO);
        assert_eq!(xor(U256::ZERO, U256::ZERO), U256::ZERO);
    }

    #[test]
    fn test_not() {
        for i in 0..u8::MAX {
            let mut expect = [0xff; 32];
            expect[31] = !i;
            assert_eq!(not(U256::from(i)), U256::from(expect))
        } 

        assert_eq!(not(U256::MAX), U256::ZERO);
        assert_eq!(not(U256::ZERO), U256::MAX);
    }
}
