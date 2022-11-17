use fvm_ipld_kamt::{AsHashedKey, HashedKey};
use std::borrow::Cow;

use {
    crate::interpreter::{StatusCode, U256},
    cid::Cid,
    fil_actors_runtime::{runtime::Runtime, ActorError},
    fvm_ipld_blockstore::Blockstore,
    fvm_ipld_kamt::Kamt,
};

/// Solidity already hashes the keys before it tries to access the KAMT.
pub struct StateHashAlgorithm;

/// Wrapper around the base U256 type so we can control the byte order in the hash, because
/// the words backing `U256` are in little endian order, and we need them in big endian for
/// the nibbles to be co-located in the tree.
impl AsHashedKey<U256, 32> for StateHashAlgorithm {
    fn as_hashed_key(key: &U256) -> Cow<HashedKey<32>> {
        let mut bs = [0u8; 32];
        key.to_big_endian(&mut bs);
        Cow::Owned(bs)
    }
}

/// The EVM stores its state as Key-Value pairs with both keys and values
/// being 256 bits long, which we store in a KAMT.
pub type StateKamt<BS> = Kamt<BS, U256, U256, StateHashAlgorithm>;

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
    pub rt: &'r mut RT,
    state: &'r mut StateKamt<BS>,
}

impl<'r, BS: Blockstore, RT: Runtime<BS>> System<'r, BS, RT> {
    pub fn new(rt: &'r mut RT, state: &'r mut StateKamt<BS>) -> anyhow::Result<Self> {
        Ok(Self { rt, state })
    }

    /// Reborrow the system with a shorter lifetime.
    #[allow(clippy::needless_lifetimes)]
    pub fn reborrow<'a>(&'a mut self) -> System<'a, BS, RT> {
        System { rt: &mut *self.rt, state: &mut *self.state }
    }

    pub fn flush_state(&mut self) -> Result<Cid, ActorError> {
        self.state.flush().map_err(|e| ActorError::illegal_state(e.to_string()))
    }

    /// Get value of a storage key.
    pub fn get_storage(&mut self, key: U256) -> Result<Option<U256>, StatusCode> {
        Ok(self.state.get(&key).map_err(|e| StatusCode::InternalError(e.to_string()))?.cloned())
    }

    /// Set value of a storage key.
    pub fn set_storage(
        &mut self,
        key: U256,
        value: Option<U256>,
    ) -> Result<StorageStatus, StatusCode> {
        let prev_value = self.get_storage(key)?;

        match (prev_value, value) {
            (None, None) => Ok(StorageStatus::Unchanged),
            (Some(_), None) => {
                self.state.delete(&key).map_err(|e| StatusCode::InternalError(e.to_string()))?;

                Ok(StorageStatus::Deleted)
            }
            (Some(p), Some(n)) if p == n => Ok(StorageStatus::Unchanged),
            (_, Some(v)) => {
                self.state.set(key, v).map_err(|e| StatusCode::InternalError(e.to_string()))?;

                if prev_value.is_none() {
                    Ok(StorageStatus::Added)
                } else {
                    Ok(StorageStatus::Modified)
                }
            }
        }
    }
}

#[cfg(test)]
mod test {
    use fvm_ipld_hamt::HashAlgorithm;
    use fvm_ipld_kamt::AsHashedKey;

    use super::{StateHashAlgorithm, U256Key};
    use crate::interpreter::U256;

    #[test]
    fn hashing_neighboring_keys_into_same_slot() {
        let k1 = U256::from(1);
        let k2 = U256::from(2);
        let h1 = StateHashAlgorithm::as_hashed_key(&k1);
        let h2 = StateHashAlgorithm::as_hashed_key(&k2);
        for i in 0..31 {
            assert_eq!(h1[i], h2[i]);
        }
        assert_ne!(h1[31], h2[31]);
    }
}
