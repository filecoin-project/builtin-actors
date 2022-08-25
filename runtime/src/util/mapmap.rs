use cid::Cid;
use fvm_ipld_blockstore::Blockstore;
use fvm_ipld_hamt::{Error, BytesKey};
use serde::de::DeserializeOwned;
use serde::Serialize;
use std::collections::BTreeMap;
use std::collections::btree_map::Entry::{Occupied, Vacant};
use crate::{make_empty_map, make_map_with_root_and_bitwidth, Keyer, Map};

// MapMap stores multiple values per key in a Hamt of Hamts
// Every element stored has a primary and secondary key
pub struct MapMap<'a, BS, V> 
{
    outer: Map<'a, BS, Cid>,
    inner_bitwidth: u32, 
    cache: BTreeMap<Vec<u8>, Map<'a, BS, V>>,
}
impl<'a, BS, V> MapMap<'a, BS, V>
where
    BS: Blockstore,
    V: Serialize + DeserializeOwned + Clone + std::cmp::PartialEq,
{

    pub fn new(bs: &'a BS, outer_bitwidth: u32, inner_bitwidth: u32) -> Self {
        MapMap {
            outer: make_empty_map(bs, outer_bitwidth),
            inner_bitwidth,
            cache: BTreeMap::<Vec<u8>, Map<BS, V>>::new(),
        }
    }

    pub fn from_root(bs: &'a BS, cid: &Cid, outer_bitwidth: u32, inner_bitwidth:u32 ) -> Result<Self, Error> {
        Ok(MapMap{
            outer: make_map_with_root_and_bitwidth(cid, bs, outer_bitwidth)?,
            inner_bitwidth,
            cache: BTreeMap::<Vec<u8>, Map<BS, V>>::new(),
        })
    }

    pub fn flush(&mut self) -> Result<Cid, Error> {
        for (k,in_map) in self.cache.iter_mut() {
            if in_map.is_empty() {
                self.outer.delete(&BytesKey(k.to_vec()))?;
            } else {
                let new_in_root = in_map.flush()?;
                self.outer.set(BytesKey(k.to_vec()), new_in_root)?;
            }
        }
        self.outer.flush()
    }

    fn load_inner_map<K1>(& mut self, k: K1)  -> Result<& mut Map<'a, BS, V>, Error> 
    where
        K1: Keyer + std::fmt::Debug,
    {
        let in_map_thunk = || -> Result<Map<BS, V>, Error> {
            // lazy to avoid ipld operations in case of cache hit
            match self.outer.get(&k.key())? {
                Some(root) => Ok(make_map_with_root_and_bitwidth::<BS, V>(root, *self.outer.store(), self.inner_bitwidth)?),
                None => Ok(make_empty_map(*self.outer.store(), self.inner_bitwidth)),
            }
        };
        let raw_k = k.key().0;
        match self.cache.entry(raw_k) {
            Occupied(entry) => Ok(entry.into_mut()),
            Vacant(entry) => Ok(entry.insert(in_map_thunk()?)),
        }
    }

    // memreplace -- lets you swap two values without triggering borrow checker
    // cloning something somewhere, doing this insert on some cloned version of the cache
    pub fn get<K1, K2>(& mut self, outside_k: K1, inside_k: K2) -> Result<Option<V>, Error> 
    where
        K1: Keyer+ std::fmt::Debug,
        K2: Keyer + std::fmt::Display,
    {
        let in_map = self.load_inner_map::<K1>(outside_k)?;
        Ok(in_map.get(&inside_k.key())?.cloned())
    }

    pub fn put<K1, K2>(&mut self, outside_k: K1, inside_k: K2, value: V) -> Result<bool, Error> 
    where
        K1: Keyer+ std::fmt::Debug,
        K2: Keyer,
    {
        let in_map = self.load_inner_map::<K1>(outside_k)?;

        if in_map.contains_key::<BytesKey>(&inside_k.key())? {
            return Ok(false)
        }
        in_map.set(inside_k.key(), value)?;
        // defer flushing cached inner map until flush call
        Ok(true)
    }

    pub fn remove<K1, K2>(&mut self, outside_k: K1, inside_k: K2) -> Result<(), Error> 
    where
        K1: Keyer + std::fmt::Debug,
        K2: Keyer,
    {
        let in_map = self.load_inner_map::<K1>(outside_k)?;
        if in_map.contains_key::<BytesKey>(&inside_k.key())? {
            in_map.delete(&inside_k.key())?;
        }

        Ok(())
    }
}
