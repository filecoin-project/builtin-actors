// Copyright 2019-2022 ChainSafe Systems
// SPDX-License-Identifier: Apache-2.0, MIT

use cid::Cid;
use fvm_ipld_blockstore::Blockstore;
use serde::de::DeserializeOwned;
use serde::Serialize;

use crate::{make_empty_map, make_map_with_root_and_bitwidth, Array, BytesKey, Map};

#[derive(thiserror::Error, Debug)]
pub enum Error<E> {
    #[error("amt: {0}")]
    Amt(#[from] fvm_ipld_amt::Error<E>),
    #[error("hamt: {0}")]
    Hamt(#[from] fvm_ipld_hamt::Error<E>),
}

/// Multimap stores multiple values per key in a Hamt of Amts.
/// The order of insertion of values for each key is retained.
pub struct Multimap<'a, BS>(Map<'a, BS, Cid>, u32);
impl<'a, BS> Multimap<'a, BS>
where
    BS: Blockstore,
{
    /// Initializes a new empty multimap.
    /// The outer_bitwidth is the width of the HAMT and the
    /// inner_bitwidth is the width of the AMTs inside of it.
    pub fn new(bs: &'a BS, outer_bitwidth: u32, inner_bitwidth: u32) -> Self {
        Self(make_empty_map(bs, outer_bitwidth), inner_bitwidth)
    }

    /// Initializes a multimap from a root Cid
    pub fn from_root(
        bs: &'a BS,
        cid: &Cid,
        outer_bitwidth: u32,
        inner_bitwidth: u32,
    ) -> Result<Self, Error<BS::Error>> {
        Ok(Self(make_map_with_root_and_bitwidth(cid, bs, outer_bitwidth)?, inner_bitwidth))
    }

    /// Retrieve root from the multimap.
    #[inline]
    pub fn root(&mut self) -> Result<Cid, Error<BS::Error>> {
        let cid = self.0.flush()?;
        Ok(cid)
    }

    /// Adds a value for a key.
    pub fn add<V>(&mut self, key: BytesKey, value: V) -> Result<(), Error<BS::Error>>
    where
        V: Serialize + DeserializeOwned,
    {
        // Get construct amt from retrieved cid or create new
        let mut arr = self
            .get::<V>(&key)?
            .unwrap_or_else(|| Array::new_with_bit_width(self.0.store(), self.1));

        // Set value at next index
        arr.set(arr.count(), value)?;

        // flush to get new array root to put in hamt
        let new_root = arr.flush()?;

        // Set hamt node to array root
        self.0.set(key, new_root)?;
        Ok(())
    }

    /// Gets the Array of value type `V` using the multimap store.
    #[inline]
    pub fn get<V>(&self, key: &[u8]) -> Result<Option<Array<'a, V, BS>>, Error<BS::Error>>
    where
        V: DeserializeOwned + Serialize,
    {
        match self.0.get(key)? {
            Some(cid) => Ok(Some(Array::load(cid, *self.0.store())?)),
            None => Ok(None),
        }
    }

    /// Removes all values for a key.
    #[inline]
    pub fn remove_all(&mut self, key: &[u8]) -> Result<(), Error<BS::Error>> {
        // Remove entry from table
        self.0.delete(key)?;

        Ok(())
    }

    /// Iterates through all values in the array at a given key.
    pub fn for_each<F, V, U>(&self, key: &[u8], f: F) -> Result<(), EitherError<U, BS::Error>>
    where
        V: Serialize + DeserializeOwned,
        F: FnMut(u64, &V) -> Result<(), U>,
    {
        if let Some(amt) = self.get::<V>(key)? {
            amt.for_each(f).map_err(|err| match err {
                fvm_ipld_amt::EitherError::User(e) => EitherError::User(e),
                fvm_ipld_amt::EitherError::Amt(e) => EitherError::MultiMap(e.into()),
            })?;
        }

        Ok(())
    }

    /// Iterates through all arrays in the multimap
    pub fn for_all<F, V, U>(&self, mut f: F) -> Result<(), EitherError<U, BS::Error>>
    where
        V: Serialize + DeserializeOwned,
        F: FnMut(&BytesKey, &Array<V, BS>) -> Result<(), U>,
    {
        self.0
            .for_each::<_, EitherError<U, BS::Error>>(|key, arr_root| {
                let arr = Array::load(arr_root, *self.0.store())
                    .map_err(|e| EitherError::MultiMap(e.into()))?;
                f(key, &arr).map_err(EitherError::User)?;
                Ok(())
            })
            .map_err(|err| match err {
                fvm_ipld_hamt::EitherError::User(e) => e,
                fvm_ipld_hamt::EitherError::Hamt(e) => EitherError::MultiMap(e.into()),
            })?;

        Ok(())
    }
}

/// This error wraps around around two different errors, either the native `Error` from `multimap`, or
/// a custom user error, returned from executing a user defined function.
#[derive(Debug, thiserror::Error)]
pub enum EitherError<U, E> {
    #[error("user: {0}")]
    User(U),
    #[error("multimap: {0}")]
    MultiMap(#[from] Error<E>),
}
