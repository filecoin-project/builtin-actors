#![allow(dead_code, clippy::missing_safety_doc)]

use crate::interpreter::U256;

use super::StatusCode;

/// Ethereum Yellow Paper (9.1)
pub const STACK_SIZE: usize = 1024;

const INITIAL_STACK_SIZE: usize = 32;

/// EVM stack.
#[derive(Clone, Debug)]
pub struct Stack {
    stack: Vec<U256>,
}

impl Stack {
    #[inline]
    pub fn new() -> Self {
        Stack { stack: Vec::with_capacity(INITIAL_STACK_SIZE) }
    }

    #[inline(always)]
    pub fn len(&self) -> usize {
        self.stack.len()
    }

    #[inline(always)]
    pub fn is_empty(&self) -> bool {
        self.stack.is_empty()
    }

    #[inline(always)]
    pub fn push_unchecked(&mut self, value: U256) {
        self.stack.push(value);
    }

    #[inline(always)]
    pub fn push(&mut self, value: U256) -> Result<(), StatusCode> {
        if self.stack.len() >= STACK_SIZE {
            Err(StatusCode::StackOverflow)
        } else {
            self.stack.push(value);
            Ok(())
        }
    }

    #[inline]
    pub fn pop_many<const S: usize>(&mut self) -> Result<&[U256; S], StatusCode> {
        if self.len() < S {
            return Err(StatusCode::StackUnderflow);
        }
        let new_len = self.len() - S;
        unsafe {
            // This is safe because:
            //
            // 1. U256 isn't drop.
            // 2. The borrow will end before we can do anything else.
            //
            // It's faster than copying these elements multiple times.
            self.stack.set_len(new_len);
            Ok(&*(self.stack.as_ptr().add(new_len) as *const [U256; S]))
        }
    }

    #[inline(always)]
    pub fn ensure_one(&self) -> Result<(), StatusCode> {
        if self.stack.len() >= STACK_SIZE {
            Err(StatusCode::StackOverflow)
        } else {
            Ok(())
        }
    }

    #[inline]
    pub fn dup(&mut self, i: usize) -> Result<(), StatusCode> {
        let len = self.stack.len();
        if len >= STACK_SIZE {
            Err(StatusCode::StackOverflow)
        } else if i > len {
            Err(StatusCode::StackUnderflow)
        } else {
            unsafe {
                // This is safe because we're careful not to alias. We're _basically_ implementing
                // "emplace", because rust still doesn't have it.
                //
                // Yes, this is faster than a get/push.
                self.stack.reserve(1);
                *self.stack.as_mut_ptr().add(len) = *self.stack.get_unchecked(len - i);
                self.stack.set_len(len + 1);
            }
            Ok(())
        }
    }

    #[inline]
    pub fn swap_top(&mut self, i: usize) -> Result<(), StatusCode> {
        let len = self.stack.len();
        if len < i {
            return Err(StatusCode::StackUnderflow);
        }
        self.stack.swap(len - i - 1, len - 1);
        Ok(())
    }

    #[inline]
    pub fn pop(&mut self) -> Result<U256, StatusCode> {
        self.stack.pop().ok_or(StatusCode::StackUnderflow)
    }

    #[inline]
    pub fn drop(&mut self) -> Result<(), StatusCode> {
        if self.stack.pop().is_some() {
            Ok(())
        } else {
            Err(StatusCode::StackUnderflow)
        }
    }
}

impl Default for Stack {
    fn default() -> Self {
        Self::new()
    }
}
