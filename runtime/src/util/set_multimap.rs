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

/// SetMultimap is a hamt with values that are also a hamt but are of the set variant.
/// This allows hash sets to be indexable by an address.
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
    BS: Blockstore + Clone,
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

    /// Puts the DealID in the hash set of the key.
    pub fn put(&mut self, key: &K, value: V) -> Result<(), ActorError> {
        // Get construct amt from retrieved cid or create new
        let mut set = self.get(key)?.unwrap_or_else(|| {
            Set::empty(self.outer.store().clone(), self.inner_config.clone(), "multimap inner")
        });

        set.put(&value)?;

        // Save and calculate new root
        let new_root = set.flush()?;

        // Set hamt node to set new root
        self.outer.set(key, new_root)?;
        Ok(())
    }

    /// Puts slice of DealIDs in the hash set of the key.
    pub fn put_many(&mut self, key: &K, values: &[V]) -> Result<(), ActorError> {
        // Get construct amt from retrieved cid or create new
        let mut set = self.get(key)?.unwrap_or_else(|| {
            Set::empty(self.outer.store().clone(), self.inner_config.clone(), "multimap inner")
        });

        for v in values {
            set.put(v)?;
        }

        // Save and calculate new root
        let new_root = set.flush()?;

        // Set hamt node to set new root
        self.outer.set(key, new_root)?;
        Ok(())
    }

    /// Gets the set at the given index of the `SetMultimap`
    #[inline]
    pub fn get(&self, key: &K) -> Result<Option<Set<BS, V>>, ActorError> {
        match self.outer.get(key)? {
            Some(cid) => Ok(Some(Set::load(
                self.outer.store().clone(),
                cid,
                self.inner_config.clone(),
                "multimap inner",
            )?)),
            None => Ok(None),
        }
    }

    /// Removes a DealID from a key hash set.
    #[inline]
    pub fn remove(&mut self, key: &K, v: V) -> Result<(), ActorError> {
        // Get construct amt from retrieved cid and return if no set exists
        let mut set = match self.get(key)? {
            Some(s) => s,
            None => return Ok(()),
        };

        set.delete(&v)?;

        // Save and calculate new root
        let new_root = set.flush()?;
        self.outer.set(key, new_root)?;
        Ok(())
    }

    /// Removes set at index.
    #[inline]
    pub fn remove_all(&mut self, key: &K) -> Result<(), ActorError> {
        // Remove entry from table
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
