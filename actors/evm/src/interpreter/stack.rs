#![allow(dead_code)]

use {crate::interpreter::StatusCode, crate::interpreter::U256};

/// Ethereum Yellow Paper (9.1)
pub const STACK_SIZE: usize = 1024;

/// EVM stack.
#[derive(Clone, Debug)]
pub struct Stack {
    sk: [U256; STACK_SIZE],
    d: usize,
}

impl Stack {
    #[inline]
    pub const fn new() -> Self {
        Stack { sk: [U256::zero(); STACK_SIZE], d: 0 }
    }

    #[inline]
    pub fn push(&mut self, v: U256) -> Result<(), StatusCode> {
        if self.d < STACK_SIZE {
            //self.sk[self.d] = v;
            unsafe {
                *self.sk.get_unchecked_mut(self.d) = v;
            }
            self.d += 1;
            Ok(())
        } else {
            Err(StatusCode::StackOverflow)
        }
    }

    #[inline]
    pub fn pop(&mut self) -> Result<U256, StatusCode> {
        if self.d > 0 {
            self.d -= 1;
            //Ok(self.sk[self.d])
            Ok(unsafe { *self.sk.get_unchecked(self.d) })
        } else {
            Err(StatusCode::StackUnderflow)
        }
    }

    #[inline]
    pub fn dup(&mut self, i: usize) -> Result<(), StatusCode> {
        if self.d > i - 1 {
            if self.d < STACK_SIZE {
                //self.sk[self.d] = self.sk[self.d - i];
                unsafe { *self.sk.get_unchecked_mut(self.d) = *self.sk.get_unchecked(self.d - i) };
                self.d += 1;
                Ok(())
            } else {
                Err(StatusCode::StackOverflow)
            }
        } else {
            Err(StatusCode::StackUnderflow)
        }
    }

    #[inline]
    pub fn swap(&mut self, i: usize) -> Result<(), StatusCode> {
        if self.d > i + 1 {
            let top = self.d - 1;
            let bottom = top - i;
            //self.sk.swap(top, bottom);
            unsafe {
                let tmp = *self.sk.get_unchecked(top);
                *self.sk.get_unchecked_mut(top) = *self.sk.get_unchecked(bottom);
                *self.sk.get_unchecked_mut(bottom) = tmp;
            }
            Ok(())
        } else {
            Err(StatusCode::StackUnderflow)
        }
    }

    #[inline]
    pub fn with<const N: usize, R, F: FnOnce(&[U256; N]) -> Result<R, StatusCode>>(
        &mut self,
        f: F,
    ) -> Result<R, StatusCode> {
        if self.d >= N {
            let top = self.d;
            self.d -= N;
            f(unsafe { &*(&self.sk[self.d..top] as *const [U256] as *const [U256; N]) })
        } else {
            Err(StatusCode::StackUnderflow)
        }
    }

    #[inline]
    pub fn with1<R, F: FnOnce(U256) -> Result<R, StatusCode>>(
        &mut self,
        f: F,
    ) -> Result<R, StatusCode> {
        if self.d >= 1 {
            unsafe {
                let r = f(*self.sk.get_unchecked(self.d - 1));
                self.d -= 1;
                r
            }
        } else {
            Err(StatusCode::StackUnderflow)
        }
    }

    #[inline]
    pub fn with2<R, F: FnOnce(U256, U256) -> Result<R, StatusCode>>(
        &mut self,
        f: F,
    ) -> Result<R, StatusCode> {
        if self.d >= 2 {
            unsafe {
                let r = f(*self.sk.get_unchecked(self.d - 1), *self.sk.get_unchecked(self.d - 2));
                self.d -= 2;
                r
            }
        } else {
            Err(StatusCode::StackUnderflow)
        }
    }

    #[inline]
    pub fn with3<R, F: FnOnce(U256, U256, U256) -> Result<R, StatusCode>>(
        &mut self,
        f: F,
    ) -> Result<R, StatusCode> {
        if self.d >= 3 {
            unsafe {
                let r = f(
                    *self.sk.get_unchecked(self.d - 1),
                    *self.sk.get_unchecked(self.d - 2),
                    *self.sk.get_unchecked(self.d - 3),
                );
                self.d -= 3;
                r
            }
        } else {
            Err(StatusCode::StackUnderflow)
        }
    }

    #[inline]
    pub fn apply<const N: usize, F: FnOnce(&[U256; N]) -> U256>(
        &mut self,
        f: F,
    ) -> Result<(), StatusCode> {
        if self.d >= N {
            let top = self.d;
            self.d -= N;
            let r = f(unsafe { &*(&self.sk[self.d..top] as *const [U256] as *const [U256; N]) });
            self.sk[self.d] = r;
            self.d += 1;
            Ok(())
        } else {
            Err(StatusCode::StackUnderflow)
        }
    }

    #[inline]
    pub fn apply1<F: FnOnce(U256) -> U256>(&mut self, f: F) -> Result<(), StatusCode> {
        if self.d >= 1 {
            unsafe {
                let r = f(*self.sk.get_unchecked(self.d - 1));
                *self.sk.get_unchecked_mut(self.d - 1) = r;
            }
            Ok(())
        } else {
            Err(StatusCode::StackUnderflow)
        }
    }

    #[inline]
    pub fn apply2<F: FnOnce(U256, U256) -> U256>(&mut self, f: F) -> Result<(), StatusCode> {
        if self.d >= 2 {
            unsafe {
                let r = f(*self.sk.get_unchecked(self.d - 1), *self.sk.get_unchecked(self.d - 2));
                *self.sk.get_unchecked_mut(self.d - 2) = r;
                self.d -= 1;
            }
            Ok(())
        } else {
            Err(StatusCode::StackUnderflow)
        }
    }

    #[inline]
    pub fn apply3<F: FnOnce(U256, U256, U256) -> U256>(&mut self, f: F) -> Result<(), StatusCode> {
        if self.d >= 3 {
            unsafe {
                let r = f(
                    *self.sk.get_unchecked(self.d - 1),
                    *self.sk.get_unchecked(self.d - 2),
                    *self.sk.get_unchecked(self.d - 3),
                );
                *self.sk.get_unchecked_mut(self.d - 3) = r;
                self.d -= 2;
            }
            Ok(())
        } else {
            Err(StatusCode::StackUnderflow)
        }
    }

    #[inline]
    pub fn len(&self) -> usize {
        self.d
    }

    #[inline]
    pub fn is_empty(&self) -> bool {
        self.d == 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stack() {
        let mut stack = Stack::new();

        let items: [u128; 4] = [0xde, 0xad, 0xbe, 0xef];

        for (i, item) in items.iter().copied().enumerate() {
            stack.push(item.into()).unwrap();
            assert_eq!(stack.len(), i + 1);
        }

        assert_eq!(stack.pop().unwrap(), U256::from(0xef));
        assert_eq!(stack.pop().unwrap(), U256::from(0xbe));
        assert_eq!(stack.pop().unwrap(), U256::from(0xad));
        assert_eq!(stack.pop().unwrap(), U256::from(0xde));
    }
}
