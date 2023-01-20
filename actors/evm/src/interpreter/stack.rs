#![allow(dead_code, clippy::missing_safety_doc)]

use fil_actors_runtime::{ActorError, AsActorError};

use crate::{interpreter::U256, EVM_CONTRACT_STACK_OVERFLOW, EVM_CONTRACT_STACK_UNDERFLOW};

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
    pub fn push(&mut self, value: U256) -> Result<(), ActorError> {
        if self.stack.len() >= STACK_SIZE {
            Err(ActorError::unchecked(EVM_CONTRACT_STACK_OVERFLOW, "stack overflow".into()))
        } else {
            self.stack.push(value);
            Ok(())
        }
    }

    #[inline]
    pub fn pop_many<const S: usize>(&mut self) -> Result<&[U256; S], ActorError> {
        if self.len() < S {
            return Err(ActorError::unchecked(
                EVM_CONTRACT_STACK_UNDERFLOW,
                "stack underflow".into(),
            ));
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
    /// Ensures at least one more item is able to be allocated on the stack.
    pub fn ensure_one(&self) -> Result<(), ActorError> {
        if self.stack.len() >= STACK_SIZE {
            Err(ActorError::unchecked(EVM_CONTRACT_STACK_OVERFLOW, "stack overflow".into()))
        } else {
            Ok(())
        }
    }

    #[inline]
    pub fn dup(&mut self, i: usize) -> Result<(), ActorError> {
        let len = self.stack.len();
        if len >= STACK_SIZE {
            Err(ActorError::unchecked(EVM_CONTRACT_STACK_OVERFLOW, "stack overflow".into()))
        } else if i > len {
            Err(ActorError::unchecked(EVM_CONTRACT_STACK_UNDERFLOW, "stack underflow".into()))
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
    pub fn swap_top(&mut self, i: usize) -> Result<(), ActorError> {
        let len = self.stack.len();
        if len <= i {
            return Err(ActorError::unchecked(
                EVM_CONTRACT_STACK_UNDERFLOW,
                "stack underflow".into(),
            ));
        }
        self.stack.swap(len - i - 1, len - 1);
        Ok(())
    }

    #[inline]
    pub fn pop(&mut self) -> Result<U256, ActorError> {
        self.stack.pop().context_code(EVM_CONTRACT_STACK_UNDERFLOW, "stack underflow")
    }

    #[inline]
    pub fn drop(&mut self) -> Result<(), ActorError> {
        if self.stack.pop().is_some() {
            Ok(())
        } else {
            Err(ActorError::unchecked(EVM_CONTRACT_STACK_UNDERFLOW, "stack underflow".into()))
        }
    }
}

impl Default for Stack {
    fn default() -> Self {
        Self::new()
    }
}

#[test]
fn test_stack_push_pop() {
    let mut stack = Stack::new();
    stack.push(1.into()).unwrap();
    stack.push(2.into()).unwrap();
    assert_eq!(stack.pop().unwrap(), 2);
    assert_eq!(stack.pop().unwrap(), 1);
}

#[test]
fn test_stack_swap() {
    let mut stack = Stack::new();
    stack.push(1.into()).unwrap();
    stack.push(2.into()).unwrap();
    stack.swap_top(1).unwrap();
    assert_eq!(stack.pop().unwrap(), 1);
    assert_eq!(stack.pop().unwrap(), 2);

    let mut stack = Stack::new();
    stack.push(1.into()).unwrap();
    stack.push(2.into()).unwrap();
    stack.push(3.into()).unwrap();
    stack.swap_top(2).unwrap();
    assert_eq!(stack.pop().unwrap(), 1);
    assert_eq!(stack.pop().unwrap(), 2);
    assert_eq!(stack.pop().unwrap(), 3);
}

#[test]
fn test_stack_swap_underflow() {
    let mut stack = Stack::new();
    assert_eq!(stack.swap_top(1).unwrap_err().exit_code(), EVM_CONTRACT_STACK_UNDERFLOW);

    stack.push(1.into()).unwrap();
    assert_eq!(stack.swap_top(1).unwrap_err().exit_code(), EVM_CONTRACT_STACK_UNDERFLOW);

    stack.push(2.into()).unwrap();
    assert_eq!(stack.swap_top(2).unwrap_err().exit_code(), EVM_CONTRACT_STACK_UNDERFLOW);
}

#[test]
fn test_stack_dup() {
    let mut stack = Stack::new();
    stack.push(1.into()).unwrap();
    stack.push(2.into()).unwrap();
    stack.dup(1).unwrap();
    assert_eq!(stack.pop().unwrap(), 2);
    stack.dup(2).unwrap();
    assert_eq!(stack.pop().unwrap(), 1);
    assert_eq!(stack.pop().unwrap(), 2);
    assert_eq!(stack.pop().unwrap(), 1);
}

#[test]
fn test_stack_dup_underflow() {
    let mut stack = Stack::new();
    assert_eq!(stack.dup(1).unwrap_err().exit_code(), EVM_CONTRACT_STACK_UNDERFLOW);
    stack.push(1.into()).unwrap();
    assert_eq!(stack.dup(2).unwrap_err().exit_code(), EVM_CONTRACT_STACK_UNDERFLOW);
}

#[test]
fn test_stack_overflow() {
    let mut stack = Stack::new();
    for i in 0..1024 {
        stack.push(i.into()).unwrap();
    }

    assert_eq!(stack.push(1024.into()).unwrap_err().exit_code(), EVM_CONTRACT_STACK_OVERFLOW);
    assert_eq!(stack.dup(1).unwrap_err().exit_code(), EVM_CONTRACT_STACK_OVERFLOW);
    stack.swap_top(1).unwrap();
    assert_eq!(stack.pop().unwrap(), 1022);
}
