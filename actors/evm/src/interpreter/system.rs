#![allow(dead_code)]

use std::fmt::Display;

use fvm_ipld_hamt::{Hash, Identity};
use serde::{Deserialize, Serialize};

use {
    crate::interpreter::{StatusCode, U256},
    cid::Cid,
    fil_actors_runtime::{runtime::Runtime, ActorError},
    fvm_ipld_blockstore::Blockstore,
    fvm_ipld_hamt::Hamt,
};

/// We can use the identity hashing because the keys we need to hash are just the right size.
type StateHashAlgorithm = Identity;

/// Wrapper around the base U256 type so we can control the byte order in the hash, because
/// the words backing `U256` are in little endian order, and we need them in big endian for
/// the nibbles to be co-located in the tree.
///
/// It would be tempting to define a `HashingAlgorithm` that takes care of this, but that's
/// not possible because `HashingAlgorithm` has to deal with any data that knows how to hash
/// itself, not just the key type of the HAMT.
#[derive(Eq, PartialEq, PartialOrd, Debug)]
pub struct U256Key(U256);

impl Hash for U256Key {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        let mut bs = [0; 32];
        self.0.to_big_endian(&mut bs);
        bs.hash(state);
    }
}

impl Serialize for U256Key {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        self.0.serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for U256Key {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        U256::deserialize(deserializer).map(U256Key)
    }
}

impl Display for U256Key {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(f)
    }
}

/// The EVM stores its state as Key-Value pairs with both keys and values
/// being 256 bits long. We store thse in a HAMT, The keys are already hashed
/// by the Solidity compiler, so we can use the identity hasher.
pub type StateHamt<BS> = Hamt<BS, U256, U256Key, StateHashAlgorithm>;

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
    state: &'r mut StateHamt<BS>,
}

impl<'r, BS: Blockstore, RT: Runtime<BS>> System<'r, BS, RT> {
    pub fn new(rt: &'r mut RT, state: &'r mut StateHamt<BS>) -> anyhow::Result<Self> {
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
        Ok(self
            .state
            .get(&U256Key(key))
            .map_err(|e| StatusCode::InternalError(e.to_string()))?
            .cloned())
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
                self.state
                    .delete(&U256Key(key))
                    .map_err(|e| StatusCode::InternalError(e.to_string()))?;

                Ok(StorageStatus::Deleted)
            }
            (Some(p), Some(n)) if p == n => Ok(StorageStatus::Unchanged),
            (_, Some(v)) => {
                self.state
                    .set(U256Key(key), v)
                    .map_err(|e| StatusCode::InternalError(e.to_string()))?;

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

    use super::{StateHashAlgorithm, U256Key};
    use crate::interpreter::U256;

    #[test]
    fn hashing_neighboring_keys_into_same_slot() {
        let k1 = U256Key(U256::from(1));
        let k2 = U256Key(U256::from(2));
        let h1 = StateHashAlgorithm::hash(&k1);
        let h2 = StateHashAlgorithm::hash(&k2);
        for i in 0..31 {
            assert_eq!(h1[i], h2[i]);
        }
        assert_ne!(h1[31], h2[31]);
    }
}
