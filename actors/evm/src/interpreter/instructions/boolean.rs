use std::cmp::Ordering;

use fil_actors_evm_shared::uints::U256;

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
    U256::from_u64((a.i256_cmp(&b) == Ordering::Less).into())
}

#[inline]
pub(crate) fn sgt(a: U256, b: U256) -> U256 {
    U256::from_u64((a.i256_cmp(&b) == Ordering::Greater).into())
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
    fn test_less_than() {
        for i in 0..u8::MAX {
            for j in 0..u8::MAX {
                let a = U256::from(i);
                let b = U256::from(j);
                assert_eq!(lt(a, b), U256::from((i < j) as u8))
            }
        }

        assert_eq!(lt(U256::ZERO, U256::ZERO), U256::ZERO);
        assert_eq!(lt(U256::ZERO, U256::MAX), U256::ONE);
        assert_eq!(lt(U256::MAX, U256::ZERO), U256::ZERO);
        assert_eq!(lt(U256::MAX, U256::MAX), U256::ZERO);
    }

    #[test]
    fn test_greater_than() {
        for i in 0..u8::MAX {
            for j in 0..u8::MAX {
                let a = U256::from(i);
                let b = U256::from(j);
                assert_eq!(gt(a, b), U256::from((i > j) as u8))
            }
        }

        assert_eq!(gt(U256::ZERO, U256::ZERO), U256::ZERO);
        assert_eq!(gt(U256::ZERO, U256::MAX), U256::ZERO);
        assert_eq!(gt(U256::MAX, U256::ZERO), U256::ONE);
        assert_eq!(gt(U256::MAX, U256::MAX), U256::ZERO);
    }

    #[test]
    fn test_sign_less_than() {
        let first = i8::MIN + 1;
        let last = i8::MAX;
        for i in first..=last {
            for j in first..=last {
                let a = if i.is_negative() { U256::from(-i).i256_neg() } else { U256::from(i) };

                let b = if j.is_negative() { U256::from(-j).i256_neg() } else { U256::from(j) };

                assert_eq!(slt(a, b), U256::from((i < j) as u8))
            }
        }

        assert_eq!(slt(U256::I128_MIN, U256::ZERO), U256::ONE);
        assert_eq!(slt(U256::ZERO, U256::from(i128::MAX).i256_neg()), U256::ZERO);
        assert_eq!(slt(U256::ZERO, U256::from(i128::MAX)), U256::ONE);
    }

    #[test]
    fn test_sign_greater_than() {
        let first = i8::MIN + 1;
        let last = i8::MAX;
        for i in first..=last {
            for j in first..=last {
                let a = if i.is_negative() { U256::from(-i).i256_neg() } else { U256::from(i) };

                let b = if j.is_negative() { U256::from(-j).i256_neg() } else { U256::from(j) };

                assert_eq!(sgt(a, b), U256::from((i > j) as u8))
            }
        }

        assert_eq!(sgt(U256::I128_MIN, U256::ZERO), U256::ZERO);
        assert_eq!(sgt(U256::ZERO, U256::from(i128::MAX).i256_neg()), U256::ONE);
        assert_eq!(sgt(U256::ZERO, U256::from(i128::MAX)), U256::ZERO);
    }

    #[test]
    fn test_eq() {
        assert_eq!(eq(U256::I128_MIN, U256::ZERO), U256::ZERO);
        assert_eq!(eq(U256::ZERO, U256::from(i128::MAX).i256_neg()), U256::ZERO);
        assert_eq!(eq(U256::ZERO, U256::from(i128::MAX)), U256::ZERO);
        assert_eq!(eq(U256::ZERO, (U256::MAX).i256_neg()), U256::ZERO);
        assert_eq!(eq(U256::ZERO, U256::ZERO), U256::ONE);
        assert_eq!(eq(U256::MAX, U256::MAX), U256::ONE);
    }

    #[test]
    fn test_iszero() {
        assert_eq!(iszero(U256::I128_MIN), U256::ZERO);
        assert_eq!(iszero(U256::MAX), U256::ZERO);
        assert_eq!(iszero(U256::MAX.i256_neg()), U256::ZERO);
        assert_eq!(iszero(U256::ONE), U256::ZERO);
        assert_eq!(iszero(U256::from(12345)), U256::ZERO);
        assert_eq!(iszero(U256::ZERO), U256::ONE);
    }

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
