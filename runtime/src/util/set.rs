// Copyright 2019-2022 ChainSafe Systems
// SPDX-License-Identifier: Apache-2.0, MIT

use cid::Cid;
use fvm_ipld_blockstore::Blockstore;

use crate::{ActorError, Config, Map2, MapKey};

/// Set is a HAMT with empty values.
pub struct Set<BS, K>(Map2<BS, K, ()>)
where
    BS: Blockstore,
    K: MapKey;

impl<BS, K> Set<BS, K>
where
    BS: Blockstore,
    K: MapKey,
{
    /// Initializes a new empty Set with the default bitwidth.
    pub fn empty(bs: BS, config: Config, name: &'static str) -> Self {
        Self(Map2::empty(bs, config, name))
    }

    /// Initializes a Set from a root Cid.
    pub fn load(
        bs: BS,
        root: &Cid,
        config: Config,
        name: &'static str,
    ) -> Result<Self, ActorError> {
        Ok(Self(Map2::load(bs, root, config, name)?))
    }

    /// Retrieve root from the Set.
    #[inline]
    pub fn flush(&mut self) -> Result<Cid, ActorError> {
        self.0.flush()
    }

    /// Adds key to the set.
    #[inline]
    pub fn put(&mut self, key: &K) -> Result<Option<()>, ActorError> {
        self.0.set(key, ())
    }

    /// Checks if key exists in the set.
    #[inline]
    pub fn has(&self, key: &K) -> Result<bool, ActorError> {
        self.0.contains_key(key)
    }

    /// Deletes key from set.
    #[inline]
    pub fn delete(&mut self, key: &K) -> Result<Option<()>, ActorError> {
        self.0.delete(key)
    }

    /// Iterates through all keys in the set.
    pub fn for_each<F>(&self, mut f: F) -> Result<(), ActorError>
    where
        F: FnMut(K) -> Result<(), ActorError>,
    {
        self.0.for_each(|s, _| f(s))
    }

    /// Collects all keys from the set into a vector.
    pub fn collect_keys(&self) -> Result<Vec<K>, ActorError> {
        let mut ret_keys = Vec::new();
        self.for_each(|k| {
            ret_keys.push(k);
            Ok(())
        })?;
        Ok(ret_keys)
    }
}
