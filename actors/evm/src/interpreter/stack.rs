#![allow(dead_code, clippy::missing_safety_doc)]

use crate::interpreter::U256;

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
        let required = self.d + space;

        if required > self.sk.len() {
            if required > STACK_SIZE {
                return false;
            }

            while required > self.sk.len() {
                self.sk.resize(2 * self.sk.len(), U256::zero());
            }
        }
        true
    }

    #[inline(always)]
    pub fn len(&self) -> usize {
        self.d
    }

    #[inline(always)]
    pub fn is_empty(&self) -> bool {
        self.d == 0
    }

    #[inline]
    pub unsafe fn get(&self, i: usize) -> &U256 {
        let pos = self.d - i - 1;
        self.sk.get_unchecked(pos)
    }

    #[inline]
    pub unsafe fn get_mut(&mut self, i: usize) -> &mut U256 {
        let pos = self.d - i - 1;
        self.sk.get_unchecked_mut(pos)
    }

    #[inline]
    pub unsafe fn push(&mut self, v: U256) {
        //*self.sk.get_unchecked_mut(self.d) = v;
        self.sk[self.d] = v;
        self.d += 1;
    }

    #[inline]
    pub unsafe fn pop(&mut self) -> U256 {
        self.d -= 1;
        //*self.sk.get_unchecked(self.d)
        self.sk[self.d]
    }

    #[inline]
    pub unsafe fn swap_top(&mut self, i: usize) {
        let top = self.d - 1;
        let pos = self.d - i - 1;
        let tmp = *self.sk.get_unchecked(top);
        *self.sk.get_unchecked_mut(top) = *self.sk.get_unchecked(pos);
        *self.sk.get_unchecked_mut(pos) = tmp;
    }
}

impl Default for Stack {
    fn default() -> Self {
        Self::new()
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
            unsafe { stack.push(item.into()) };
            assert_eq!(stack.len(), i + 1);
        }

        assert_eq!(unsafe { *stack.get(2) }, U256::from(0xad));
        assert_eq!(unsafe { stack.pop() }, U256::from(0xef));
        assert_eq!(unsafe { *stack.get(2) }, U256::from(0xde));
    }
}
