// Copyright 2019-2022 ChainSafe Systems
// SPDX-License-Identifier: Apache-2.0, MIT

use cid::Cid;
use fvm_ipld_blockstore::Blockstore;
use fvm_ipld_encoding::tuple::*;
use fvm_shared::address::{Address, Protocol};
use fvm_shared::ActorID;

use fil_actors_runtime::{
    actor_error, ActorError, Map2, DEFAULT_HAMT_CONFIG, FIRST_NON_SINGLETON_ADDR,
};

#[derive(Serialize_tuple, Deserialize_tuple, Clone, Debug)]
pub struct State {
    /// HAMT[Address]ActorID
    pub address_map: Cid,
    pub next_id: ActorID,
    pub network_name: String,
}

pub type AddressMap<BS> = Map2<BS, Address, ActorID>;

impl State {
    pub fn new<BS: Blockstore>(store: &BS, network_name: String) -> Result<Self, ActorError> {
        let empty = AddressMap::flush_empty(store, DEFAULT_HAMT_CONFIG)?;
        Ok(Self { address_map: empty, next_id: FIRST_NON_SINGLETON_ADDR, network_name })
    }

    /// Maps argument addresses to to a new or existing actor ID.
    /// With no delegated address, or if the delegated address is not already mapped,
    /// allocates a new ID address and maps both to it.
    /// If the delegated address is already present, maps the robust address to that actor ID.
    /// Fails if the robust address is already mapped. The assignment of an ID to an address is one-time-only, even if the actor at that ID is deleted.
    /// Returns the actor ID and a boolean indicating whether or not the actor already exists.
    pub fn map_addresses_to_id<BS: Blockstore>(
        &mut self,
        store: &BS,
        robust_addr: &Address,
        delegated_addr: Option<&Address>,
    ) -> Result<(ActorID, bool), ActorError> {
        let mut map = AddressMap::load(store, &self.address_map, DEFAULT_HAMT_CONFIG, "addresses")?;
        let (id, existing) = if let Some(delegated_addr) = delegated_addr {
            // If there's a delegated address, either recall the already-mapped actor ID or
            // create and map a new one.
            if let Some(existing_id) = map.get(delegated_addr)? {
                (*existing_id, true)
            } else {
                let new_id = self.next_id;
                self.next_id += 1;
                map.set(delegated_addr, new_id)?;
                (new_id, false)
            }
        } else {
            // With no delegated address, always create a new actor ID.
            let new_id = self.next_id;
            self.next_id += 1;
            (new_id, false)
        };

        // Map the robust address to the ID, failing if it's already mapped to anything.
        let is_new = map.set_if_absent(robust_addr, id)?;
        if !is_new {
            return Err(actor_error!(
                forbidden,
                "robust address {} is already allocated in the address map",
                robust_addr
            ));
        }
        self.address_map = map.flush()?;
        Ok((id, existing))
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
    ) -> Result<Option<Address>, ActorError> {
        if addr.protocol() == Protocol::ID {
            return Ok(Some(*addr));
        }
        let map = AddressMap::load(store, &self.address_map, DEFAULT_HAMT_CONFIG, "addresses")?;
        let found = map.get(addr)?;
        Ok(found.copied().map(Address::new_id))
    }
}
