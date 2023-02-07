use derive_more::{Deref, DerefMut};

use crate::EVM_WORD_SIZE;

const PAGE_SIZE: usize = 4 * 1024;

#[derive(Clone, Debug, Deref, DerefMut)]
pub struct Memory(Vec<u8>);

impl Default for Memory {
    fn default() -> Self {
        Self(Vec::with_capacity(PAGE_SIZE))
    }
}

impl Memory {
    #[inline]
    /// Reserve extra pages of memory
    fn reserve_pages(&mut self, pages: usize) {
        self.0.reserve((PAGE_SIZE * pages) - self.0.len());
    }

    #[inline]
    /// Grows memory to a new size, reserving extra pages as-needed.
    /// `new_size` may be unaligned.
    ///
    /// Do nothing if `new_size` doesn't grow memory.
    pub fn grow(&mut self, mut new_size: usize) {
        if new_size <= self.len() {
            return;
        }

        // Align to the next u256.
        // Guaranteed to not overflow.
        let alignment = new_size % EVM_WORD_SIZE;
        if alignment > 0 {
            new_size += EVM_WORD_SIZE - alignment;
        }

        // Reserve any new pages.
        let cap = self.0.capacity();
        if new_size > cap {
            let required_pages = (new_size + PAGE_SIZE - 1) / PAGE_SIZE;
            self.reserve_pages(required_pages);
        }

        debug_assert_eq!(new_size % 32, 0, "MSIZE depends that memory is aligned to 32 bytes");
        // Grow to new aligned size.
        self.0.resize(new_size, 0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn grow() {
        let mut mem = Memory::default();
        mem.grow(PAGE_SIZE * 2 + 1);
        assert_eq!(mem.len(), PAGE_SIZE * 2 + EVM_WORD_SIZE);
        assert_eq!(mem.capacity(), PAGE_SIZE * 3);
    }
}
