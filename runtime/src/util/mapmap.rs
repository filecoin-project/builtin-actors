use cid::Cid;
use fvm_ipld_blockstore::Blockstore;
use fvm_ipld_hamt::{Error, BytesKey};
use serde::de::DeserializeOwned;
use serde::Serialize;

use crate::{make_empty_map, make_map_with_root_and_bitwidth, Keyer, Map};

// MapMap stores multiple values per key in a Hamt of Hamts
// Every element stored has a primary and secondary key
pub struct MapMap<'a, BS>(Map<'a, BS, Cid>, u32);
impl<'a, BS> MapMap<'a, BS>
where
    BS: Blockstore,
{

    pub fn new(bs: &'a BS, outer_bitwidth: u32, inner_bitwidth: u32) -> Self {
        Self(make_empty_map(bs, outer_bitwidth), inner_bitwidth)
    }

    pub fn from_root(bs: &'a BS, cid: &Cid, outer_bitwidth: u32, inner_bitwidth:u32 ) -> Result<Self, Error> {
        Ok(Self(make_map_with_root_and_bitwidth(cid, bs, outer_bitwidth)?, inner_bitwidth))
    }

    pub fn root(&mut self) -> Result<Cid, Error> {
        self.0.flush()
    }

    pub fn get<K1, K2, V>(&self, outside_k: K1, inside_k: K2) -> Result<Option<V>, Error> 
    where
        V: Serialize + DeserializeOwned + Clone,
        K1: Keyer,
        K2: Keyer,
    {
        let in_map = match self.0.get(&outside_k.key())? {
            Some(root) => make_map_with_root_and_bitwidth::<BS, V>(root, *self.0.store(), self.1)?,
            None => make_empty_map(*self.0.store(), self.1),
        };

        Ok(in_map.get(&inside_k.key())?.cloned())
    }

    pub fn put<K1, K2, V>(&mut self, outside_k: K1, inside_k: K2, value: V) -> Result<(), Error> 
    where
        V: Serialize + DeserializeOwned + PartialEq,
        K1: Keyer,
        K2: Keyer + std::fmt::Display,
    {
        let mut in_map = match self.0.get(&outside_k.key())? {
            Some(root) => make_map_with_root_and_bitwidth::<BS, V>(root, *self.0.store(), self.1)?,
            None => make_empty_map(*self.0.store(), self.1),
        };

        if in_map.contains_key::<BytesKey>(&inside_k.key())? {
            return Err(Error::from(format!("put not allowed on existing key {}", &inside_k)))
        }
        in_map.set(inside_k.key(), value)?;
        let new_root = in_map.flush()?;

        self.0.set(outside_k.key(), new_root)?;
        Ok(())
    }

    pub fn remove<K1, K2, V>(&mut self, outside_k: K1, inside_k: K2) -> Result<(), Error> 
    where
        K1: Keyer,
        K2: Keyer,
        V: Serialize + DeserializeOwned,
    {
        let mut in_map = match self.0.get(&outside_k.key())? {
            Some(root) => make_map_with_root_and_bitwidth::<BS, V>(root, *self.0.store(), self.1)?,
            None => return Ok(()),
        };
        if in_map.contains_key::<BytesKey>(&inside_k.key())? {
            in_map.delete(&inside_k.key())?;
        }
        if in_map.is_empty() {
            self.0.delete(&outside_k.key())?;
        } else {
            let new_root = in_map.flush()?;
            self.0.set(outside_k.key(), new_root)?;
        }
        Ok(())
    }

}

