// Copyright 2019-2022 ChainSafe Systems
// SPDX-License-Identifier: Apache-2.0, MIT

#[cfg(feature = "m2-native")]
use cid::multihash::Code;
use cid::Cid;
use fil_actors_runtime::{
    actor_error, make_empty_map, make_map_with_root_and_bitwidth, ActorError, AsActorError,
    FIRST_NON_SINGLETON_ADDR,
};
use fvm_ipld_blockstore::Blockstore;
use fvm_ipld_encoding::tuple::*;
use fvm_ipld_encoding::Cbor;
#[cfg(feature = "m2-native")]
use fvm_ipld_encoding::CborStore;
use fvm_shared::address::{Address, Protocol};
use fvm_shared::error::ExitCode;
use fvm_shared::{ActorID, HAMT_BIT_WIDTH};

/// State is reponsible for creating
#[derive(Serialize_tuple, Deserialize_tuple, Clone, Debug)]
pub struct State {
    pub address_map: Cid,
    pub next_id: ActorID,
    pub network_name: String,
    #[cfg(feature = "m2-native")]
    pub installed_actors: Cid,
}

impl State {
    pub fn new<BS: Blockstore>(store: &BS, network_name: String) -> Result<Self, ActorError> {
        let empty_map = make_empty_map::<_, ()>(store, HAMT_BIT_WIDTH)
            .flush()
            .context_code(ExitCode::USR_ILLEGAL_STATE, "failed to create empty map")?;
        #[cfg(feature = "m2-native")]
        let installed_actors = store.put_cbor(&Vec::<Cid>::new(), Code::Blake2b256).context_code(
            ExitCode::USR_ILLEGAL_STATE,
            "failed to create installed actors object",
        )?;
        Ok(Self {
            address_map: empty_map,
            next_id: FIRST_NON_SINGLETON_ADDR,
            network_name,
            #[cfg(feature = "m2-native")]
            installed_actors,
        })
    }

    /// Maps argument addresses to to a new or existing actor ID.
    /// With no delegated address, or if the delegated address is not already mapped,
    /// allocates a new ID address and maps both to it.
    /// If the delegated address is already present, maps the robust address to that actor ID.
    /// Fails if the robust address is already mapped, providing tombstone.
    ///
    /// Returns the nwe or existing actor ID.
    pub fn map_addresses_to_id<BS: Blockstore>(
        &mut self,
        store: &BS,
        robust_addr: &Address,
        delegated_addr: Option<&Address>,
    ) -> Result<ActorID, ActorError> {
        let mut map = make_map_with_root_and_bitwidth(&self.address_map, store, HAMT_BIT_WIDTH)
            .context_code(ExitCode::USR_ILLEGAL_STATE, "failed to load address map")?;
        let id = if let Some(delegated_addr) = delegated_addr {
            // If there's a delegated address, either recall the already-mapped actor ID or
            // create and map a new one.
            let delegated_key = delegated_addr.to_bytes().into();
            if let Some(existing_id) = map
                .get(&delegated_key)
                .context_code(ExitCode::USR_ILLEGAL_STATE, "failed to lookup delegated address")?
            {
                *existing_id
            } else {
                let new_id = self.next_id;
                self.next_id += 1;
                map.set(delegated_key, new_id)
                    .context_code(ExitCode::USR_ILLEGAL_STATE, "failed to map delegated address")?;
                new_id
            }
        } else {
            // With no delegated address, always create a new actor ID.
            let new_id = self.next_id;
            self.next_id += 1;
            new_id
        };

        // Map the robust address to the ID, failing if it's already mapped to anything.
        let is_new = map
            .set_if_absent(robust_addr.to_bytes().into(), id)
            .context_code(ExitCode::USR_ILLEGAL_STATE, "failed to map robust address")?;
        if !is_new {
            return Err(actor_error!(
                forbidden,
                "robust address {} is already allocated in the address map",
                robust_addr
            ));
        }
        self.address_map =
            map.flush().context_code(ExitCode::USR_ILLEGAL_STATE, "failed to store address map")?;
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
    ) -> Result<Option<Address>, ActorError> {
        if addr.protocol() == Protocol::ID {
            return Ok(Some(*addr));
        }

        let map = make_map_with_root_and_bitwidth(&self.address_map, store, HAMT_BIT_WIDTH)
            .context_code(ExitCode::USR_ILLEGAL_STATE, "failed to load address map")?;

        let found = map
            .get(&addr.to_bytes())
            .context_code(ExitCode::USR_ILLEGAL_STATE, "failed to get address entry")?;
        Ok(found.copied().map(Address::new_id))
    }

    /// Check to see if an actor is already installed
    #[cfg(feature = "m2-native")]
    pub fn is_installed_actor<BS: Blockstore>(
        &self,
        store: &BS,
        cid: &Cid,
    ) -> Result<bool, ActorError> {
        let installed: Vec<Cid> = match store
            .get_cbor(&self.installed_actors)
            .context_code(ExitCode::USR_ILLEGAL_STATE, "failed to load installed actor list")?
        {
            Some(v) => v,
            None => Vec::new(),
        };
        Ok(installed.contains(cid))
    }

    /// Adds a new code Cid to the list of installed actors.
    #[cfg(feature = "m2-native")]
    pub fn add_installed_actor<BS: Blockstore>(
        &mut self,
        store: &BS,
        cid: Cid,
    ) -> Result<(), ActorError> {
        let mut installed: Vec<Cid> = match store
            .get_cbor(&self.installed_actors)
            .context_code(ExitCode::USR_ILLEGAL_STATE, "failed to load installed actor list")?
        {
            Some(v) => v,
            None => Vec::new(),
        };
        installed.push(cid);
        self.installed_actors = store
            .put_cbor(&installed, Code::Blake2b256)
            .context_code(ExitCode::USR_ILLEGAL_STATE, "failed to save installed actor list")?;
        Ok(())
    }
}

impl Cbor for State {}
