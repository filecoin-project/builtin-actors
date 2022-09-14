#![allow(dead_code)]

use crate::interpreter::U256;
use std::cmp::max;

/// Ethereum Yellow Paper (9.1)
pub const STACK_SIZE: usize = 1024;

const INITIAL_STACK_SIZE: usize = 32;

/// EVM stack.
#[derive(Clone, Debug)]
pub struct Stack {
    sk: Vec<U256>,
    d: usize,
}

impl Stack {
    #[inline]
    pub fn new() -> Self {
        Stack { sk: Vec::from([U256::zero(); INITIAL_STACK_SIZE]), d: 0 }
    }

    #[inline]
    pub fn require(&self, required: usize) -> bool {
        required <= self.d
    }

    #[inline]
    pub fn ensure(&mut self, space: usize) -> bool {
        if self.d + space > STACK_SIZE {
            return false;
        }

        if self.d + space >= self.sk.len() {
            self.sk.resize(max(2 * self.sk.len(), self.d + space), U256::zero());
        }
        true
    }

    #[inline]
    pub fn get(&self, i: usize) -> &U256 {
        let pos = self.d - i - 1;
        unsafe { self.sk.get_unchecked(pos) }
    }

    #[inline]
    pub fn get_mut(&mut self, i: usize) -> &mut U256 {
        let pos = self.d - i - 1;
        unsafe { self.sk.get_unchecked_mut(pos) }
    }

    #[inline(always)]
    pub const fn len(&self) -> usize {
        self.d
    }

    #[inline(always)]
    pub fn is_empty(&self) -> bool {
        self.d == 0
    }

    #[inline]
    pub fn push(&mut self, v: U256) {
        unsafe {
            *self.sk.get_unchecked_mut(self.d) = v;
        }
        self.d += 1;
    }

    #[inline]
    pub fn pop(&mut self) -> U256 {
        self.d -= 1;
        unsafe { *self.sk.get_unchecked(self.d) }
    }

    #[inline]
    pub fn swap_top(&mut self, i: usize) {
        let top = self.d - 1;
        let pos = self.d - i - 1;
        unsafe {
            let tmp = *self.sk.get_unchecked(top);
            *self.sk.get_unchecked_mut(top) = *self.sk.get_unchecked(pos);
            *self.sk.get_unchecked_mut(pos) = tmp;
        }
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
            stack.push(item.into());
            assert_eq!(stack.len(), i + 1);
        }

        assert_eq!(*stack.get(2), U256::from(0xad));
        assert_eq!(stack.pop(), U256::from(0xef));
        assert_eq!(*stack.get(2), U256::from(0xde));
    }
}
