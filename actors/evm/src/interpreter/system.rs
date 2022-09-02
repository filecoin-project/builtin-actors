#![allow(dead_code)]

use {
    crate::interpreter::{StatusCode, U256},
    cid::Cid,
    fil_actors_runtime::{runtime::Runtime, ActorError},
    fvm_ipld_blockstore::Blockstore,
    fvm_ipld_hamt::Hamt,
    std::cell::RefCell,
};

#[derive(Clone, Copy, Debug)]
pub enum StorageStatus {
    /// The value of a storage item has been left unchanged: 0 -> 0 and X -> X.
    Unchanged,
    /// The value of a storage item has been modified: X -> Y.
    Modified,
    /// A storage item has been modified after being modified before: X -> Y -> Z.
    ModifiedAgain,
    /// A new storage item has been added: 0 -> X.
    Added,
    /// A storage item has been deleted: X -> 0.
    Deleted,
}

/// Platform Abstraction Layer
/// that bridges the FVM world to EVM world
pub struct System<'r, BS: Blockstore, RT: Runtime<BS>> {
    pub rt: &'r RT,
    state: RefCell<Hamt<&'r BS, U256, U256>>,
}

impl<'r, BS: Blockstore, RT: Runtime<BS>> System<'r, BS, RT> {
    pub fn new(rt: &'r RT, state_cid: Cid) -> anyhow::Result<Self> {
        Ok(Self { rt, state: RefCell::new(Hamt::load(&state_cid, rt.store())?) })
    }

    pub fn flush_state(&self) -> Result<Cid, ActorError> {
        self.state.borrow_mut().flush().map_err(|e| ActorError::illegal_state(e.to_string()))
    }

    /// Get value of a storage key.
    pub fn get_storage(&self, key: U256) -> Result<Option<U256>, StatusCode> {
        let mut key_bytes = [0u8; 32];
        key.to_big_endian(&mut key_bytes);

        Ok(self
            .state
            .borrow()
            .get(&key)
            .map_err(|e| StatusCode::InternalError(e.to_string()))?
            .cloned())
    }

    /// Set value of a storage key.
    pub fn set_storage(&self, key: U256, value: Option<U256>) -> Result<StorageStatus, StatusCode> {
        let mut key_bytes = [0u8; 32];
        key.to_big_endian(&mut key_bytes);

        let prev_value = self
            .state
            .borrow()
            .get(&key)
            .map_err(|e| StatusCode::InternalError(e.to_string()))?
            .cloned();

        let mut storage_status =
            if prev_value == value { StorageStatus::Unchanged } else { StorageStatus::Modified };

        if value.is_none() {
            self.state
                .borrow_mut()
                .delete(&key)
                .map_err(|e| StatusCode::InternalError(e.to_string()))?;
            storage_status = StorageStatus::Deleted;
        } else {
            self.state
                .borrow_mut()
                .set(key, value.unwrap())
                .map_err(|e| StatusCode::InternalError(e.to_string()))?;
        }

        Ok(storage_status)
    }
}
