// Copyright 2021-2023 Protocol Labs
// SPDX-License-Identifier: Apache-2.0, MIT
use std::cell::RefCell;
use std::collections::HashMap;

use anyhow::Result;
use cid::Cid;

use fvm_ipld_blockstore::Blockstore;

/// Stats for a [MemoryBlockstore] this indicates the amount of read and written data
/// to the wrapped store.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct BSStats {
    /// Number of reads
    pub r: usize,
    /// Number of writes
    pub w: usize,
    /// Bytes Read
    pub br: usize,
    /// Bytes Written
    pub bw: usize,
}

/// Wrapper around `Blockstore` to tracking reads and writes for verification.
/// This struct should only be used for testing.
#[derive(Debug, Default)]
pub struct MemoryBlockstore {
    blocks: RefCell<HashMap<Cid, Vec<u8>>>,
    pub stats: RefCell<BSStats>,
}

impl MemoryBlockstore {
    pub fn new() -> Self {
        Self { blocks: Default::default(), stats: Default::default() }
    }
}

impl Blockstore for MemoryBlockstore {
    fn get(&self, cid: &Cid) -> Result<Option<Vec<u8>>> {
        let mut stats = self.stats.borrow_mut();
        stats.r += 1;

        let bytes = self.blocks.borrow().get(cid).cloned();

        if let Some(bytes) = &bytes {
            stats.br += bytes.len();
        }
        Ok(bytes)
    }
    fn has(&self, cid: &Cid) -> Result<bool> {
        self.stats.borrow_mut().r += 1;

        Ok(self.blocks.borrow().contains_key(cid))
    }

    fn put_keyed(&self, k: &Cid, block: &[u8]) -> Result<()> {
        let mut stats = self.stats.borrow_mut();
        stats.w += 1;
        stats.bw += block.len();

        self.blocks.borrow_mut().insert(*k, block.into());
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use fvm_ipld_blockstore::Block;
    use multihash::Code;

    #[test]
    fn basic_tracking_store() {
        let tr_store = MemoryBlockstore::new();
        assert_eq!(*tr_store.stats.borrow(), BSStats::default());

        let block = Block::new(0x55, &b"foobar"[..]);
        tr_store.get(&block.cid(Code::Blake2b256)).unwrap();
        assert_eq!(*tr_store.stats.borrow(), BSStats { r: 1, ..Default::default() });

        let put_cid = tr_store.put(Code::Sha2_256, &block).unwrap();
        assert_eq!(tr_store.get(&put_cid).unwrap().as_deref(), Some(block.data));
        assert_eq!(
            *tr_store.stats.borrow(),
            BSStats { r: 2, br: block.len(), w: 1, bw: block.len() }
        );

        let block2 = Block::new(0x55, &b"b2"[..]);
        let block3 = Block::new(0x55, &b"b3"[..]);
        tr_store.put_many(vec![block2, block3].into_iter().map(|b| (Code::Sha2_256, b))).unwrap();

        let total_len = block.len() + block2.len() + block3.len();

        // Read and assert blocks and tracking stats
        assert_eq!(
            tr_store.get(&block2.cid(Code::Sha2_256)).unwrap().as_deref(),
            Some(block2.data)
        );
        assert_eq!(
            *tr_store.stats.borrow(),
            BSStats { r: 3, br: total_len - block3.len(), w: 3, bw: total_len }
        );
        assert_eq!(
            tr_store.get(&block3.cid(Code::Sha2_256)).unwrap().as_deref(),
            Some(block3.data)
        );
        assert_eq!(*tr_store.stats.borrow(), BSStats { r: 4, br: total_len, w: 3, bw: total_len });
    }
}
