#![allow(dead_code)]

use {crate::interpreter::U256, crate::interpreter::StatusCode};

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
        Stack {
            sk: [U256::zero(); STACK_SIZE],
            d: 0,
        }
    }

    #[inline]
    pub fn push(&mut self, v: U256) -> Result<(), StatusCode> {
        if self.d < STACK_SIZE {
            self.sk[self.d] = v;
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
            Ok(self.sk[self.d])
        } else {
            Err(StatusCode::StackUnderflow)
        }
    }

    #[inline]
    pub fn peek(&self, i: usize) -> Result<U256, StatusCode> {
        if self.d > i {
            Ok(self.sk[self.d-i])
        } else {
            Err(StatusCode::StackUnderflow)
        }
    }

    #[inline]
    pub fn swap(&mut self, i: usize) -> Result<(), StatusCode> {
        if self.d > i {
            let tmp = self.sk[self.d];
            self.sk[self.d] = self.sk[self.d-i];
            self.sk[self.d-i] = tmp;
            Ok(())
        } else {
            Err(StatusCode::StackUnderflow)
        }
    }

    #[inline]
    pub fn with<const N: usize, R, F: FnOnce(&[U256; N]) -> Result<R, StatusCode>>(&mut self, f: F) -> Result<R, StatusCode> {
        if self.d >= N {
            let top = self.d;
            self.d -= N;
            f(unsafe { &*(&self.sk[self.d..top] as *const [U256] as *const [U256; N]) })
        } else {
            Err(StatusCode::StackUnderflow)
        }
    }

    #[inline]
    pub fn apply<const N: usize, F: FnOnce(&[U256; N]) -> U256>(&mut self, f: F) -> Result<(), StatusCode> {
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
    pub fn len(&self) -> usize {
        self.d
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
        assert_eq!(stack.pop().unwrap(), U256::from(0xab));
        assert_eq!(stack.pop().unwrap(), U256::from(0xde));
    }
}
