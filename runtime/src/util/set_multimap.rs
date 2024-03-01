// Copyright 2019-2022 ChainSafe Systems
// SPDX-License-Identifier: Apache-2.0, MIT

use std::marker::PhantomData;

use cid::Cid;
use fvm_ipld_blockstore::Blockstore;

use crate::{ActorError, Config, Map2, MapKey};

use super::Set;

pub struct SetMultimapConfig {
    pub outer: Config,
    pub inner: Config,
}

/// SetMultimap is a HAMT with values that are also a HAMT treated as a set of keys.
pub struct SetMultimap<BS, K, V>
where
    BS: Blockstore,
    K: MapKey,
    V: MapKey,
{
    outer: Map2<BS, K, Cid>,
    inner_config: Config,
    value_type: PhantomData<V>,
}

impl<BS, K, V> SetMultimap<BS, K, V>
where
    BS: Blockstore,
    K: MapKey,
    V: MapKey,
{
    /// Initializes a new empty SetMultimap.
    pub fn empty(bs: BS, config: SetMultimapConfig, name: &'static str) -> Self {
        Self {
            outer: Map2::empty(bs, config.outer, name),
            inner_config: config.inner,
            value_type: Default::default(),
        }
    }

    /// Initializes a SetMultimap from a root Cid.
    pub fn load(
        bs: BS,
        root: &Cid,
        config: SetMultimapConfig,
        name: &'static str,
    ) -> Result<Self, ActorError> {
        Ok(Self {
            outer: Map2::load(bs, root, config.outer, name)?,
            inner_config: config.inner,
            value_type: Default::default(),
        })
    }

    /// Retrieve root from the SetMultimap.
    #[inline]
    pub fn flush(&mut self) -> Result<Cid, ActorError> {
        self.outer.flush()
    }

    /// Puts a value in the set associated with a key.
    pub fn put(&mut self, key: &K, value: V) -> Result<(), ActorError> {
        // Load HAMT from retrieved cid or create a new empty one.
        let mut inner = self.get(key)?.unwrap_or_else(|| {
            Set::empty(self.outer.store(), self.inner_config.clone(), "multimap inner")
        });

        inner.put(&value)?;
        let new_root = inner.flush()?;
        self.outer.set(key, new_root)?;
        Ok(())
    }

    /// Puts slice of values in the hash set associated with a key.
    pub fn put_many(&mut self, key: &K, values: &[V]) -> Result<(), ActorError> {
        let mut inner = self.get(key)?.unwrap_or_else(|| {
            Set::empty(self.outer.store(), self.inner_config.clone(), "multimap inner")
        });

        for v in values {
            inner.put(v)?;
        }
        let new_root = inner.flush()?;
        self.outer.set(key, new_root)?;
        Ok(())
    }

    /// Gets the set of values for a key.
    #[inline]
    pub fn get(&self, key: &K) -> Result<Option<Set<&BS, V>>, ActorError> {
        match self.outer.get(key)? {
            Some(cid) => Ok(Some(Set::load(
                self.outer.store(),
                cid,
                self.inner_config.clone(),
                "multimap inner",
            )?)),
            None => Ok(None),
        }
    }

    /// Removes a value from the set associated with a key, if it was present.
    #[inline]
    pub fn remove(&mut self, key: &K, value: V) -> Result<(), ActorError> {
        let mut set = match self.get(key)? {
            Some(s) => s,
            None => return Ok(()),
        };

        set.delete(&value)?;
        let new_root = set.flush()?;
        self.outer.set(key, new_root)?;
        Ok(())
    }

    /// Removes set at index.
    #[inline]
    pub fn remove_all(&mut self, key: &K) -> Result<(), ActorError> {
        self.outer.delete(key)?;
        Ok(())
    }

    /// Iterates over all keys.
    pub fn for_each<F>(&self, mut f: F) -> Result<(), ActorError>
    where
        F: FnMut(K, &Cid) -> Result<(), ActorError>,
    {
        self.outer.for_each(|k, v| f(k, v))
    }

    /// Iterates values for a key.
    pub fn for_each_in<F>(&self, key: &K, f: F) -> Result<(), ActorError>
    where
        F: FnMut(V) -> Result<(), ActorError>,
    {
        // Get construct amt from retrieved cid and return if no set exists
        let set = match self.get(key)? {
            Some(s) => s,
            None => return Ok(()),
        };

        set.for_each(f)
    }
}
