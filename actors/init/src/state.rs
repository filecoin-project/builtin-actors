// Copyright 2019-2022 ChainSafe Systems
// SPDX-License-Identifier: Apache-2.0, MIT

use anyhow::anyhow;
use cid::multihash::Code;
use cid::Cid;
use fil_actors_runtime::{
    make_empty_map, make_map_with_root_and_bitwidth, FIRST_NON_SINGLETON_ADDR,
};
use fvm_ipld_blockstore::Blockstore;
use fvm_ipld_encoding::tuple::*;
use fvm_ipld_encoding::Cbor;
use fvm_ipld_encoding::CborStore;
use fvm_ipld_hamt::Error as HamtError;
use fvm_shared::address::{Address, Protocol};
use fvm_shared::{ActorID, HAMT_BIT_WIDTH};

/// State is reponsible for creating
#[derive(Serialize_tuple, Deserialize_tuple)]
pub struct State {
    pub address_map: Cid,
    pub next_id: ActorID,
    pub network_name: String,
    pub installed_actors: Cid,
}

impl State {
    pub fn new<BS: Blockstore>(store: &BS, network_name: String) -> anyhow::Result<Self> {
        let empty_map = make_empty_map::<_, ()>(store, HAMT_BIT_WIDTH)
            .flush()
            .map_err(|e| anyhow!("failed to create empty map: {}", e))?;
        let installed_actors = store.put_cbor(&Vec::<Cid>::new(), Code::Blake2b256)?;
        Ok(Self {
            address_map: empty_map,
            next_id: FIRST_NON_SINGLETON_ADDR,
            network_name,
            installed_actors,
        })
    }

    /// Allocates a new ID address and stores a mapping of the argument address to it.
    /// Returns the newly-allocated address.
    pub fn map_address_to_new_id<BS: Blockstore>(
        &mut self,
        store: &BS,
        addr: &Address,
    ) -> Result<ActorID, HamtError> {
        let id = self.next_id;
        self.next_id += 1;

        let mut map = make_map_with_root_and_bitwidth(&self.address_map, store, HAMT_BIT_WIDTH)?;
        map.set(addr.to_bytes().into(), id)?;
        self.address_map = map.flush()?;

        Ok(id)
    }

    /// ResolveAddress resolves an address to an ID-address, if possible.
    /// If the provided address is an ID address, it is returned as-is.
    /// This means that mapped ID-addresses (which should only appear as values, not keys) and
    /// singleton actor addresses (which are not in the map) pass through unchanged.
    ///
    /// Returns an ID-address and `true` if the address was already an ID-address or was resolved
    /// in the mapping.
    /// Returns an undefined address and `false` if the address was not an ID-address and not found
    /// in the mapping.
    /// Returns an error only if state was inconsistent.
    pub fn resolve_address<BS: Blockstore>(
        &self,
        store: &BS,
        addr: &Address,
    ) -> anyhow::Result<Option<Address>> {
        if addr.protocol() == Protocol::ID {
            return Ok(Some(*addr));
        }

        let map = make_map_with_root_and_bitwidth(&self.address_map, store, HAMT_BIT_WIDTH)?;

        Ok(map.get(&addr.to_bytes())?.copied().map(Address::new_id))
    }

    /// Check to see if an actor is already installed
    pub fn is_installed_actor<BS: Blockstore>(&self, store: &BS, cid: &Cid) -> anyhow::Result<bool> {
        let installed: Vec<Cid> = match store.get_cbor(&self.installed_actors)? {
            Some(v) => v,
            None => Vec::new(),
        };
        Ok(installed.contains(cid))
    }

    /// Adds a new code Cid to the list of installed actors.
    pub fn add_installed_actor<BS: Blockstore>(
        &mut self,
        store: &BS,
        cid: Cid,
    ) -> anyhow::Result<()> {
        let mut installed: Vec<Cid> = match store.get_cbor(&self.installed_actors)? {
            Some(v) => v,
            None => Vec::new(),
        };
        installed.push(cid);
        self.installed_actors = store.put_cbor(&installed, Code::Blake2b256)?;
        Ok(())
    }
}

impl Cbor for State {}
