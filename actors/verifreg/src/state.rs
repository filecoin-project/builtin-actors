// Copyright 2019-2022 ChainSafe Systems
// SPDX-License-Identifier: Apache-2.0, MIT

use cid::Cid;
use fil_actors_runtime::{make_empty_map, MapMap};
use fvm_ipld_blockstore::Blockstore;
use fvm_ipld_encoding::tuple::*;
use fvm_ipld_encoding::Cbor;
use fvm_shared::address::Address;
use fvm_shared::HAMT_BIT_WIDTH;
use fvm_shared::sector::SectorID;
use fvm_shared::clock::{ChainEpoch};

#[derive(Serialize_tuple, Deserialize_tuple)]
pub struct State {
    pub root_key: Address,
    pub verifiers: Cid,
    pub verified_clients: Cid,
    pub remove_data_cap_proposal_ids: Cid,
    pub allocations: Cid, // HAMT[Address]HAMT[AllocationID]Allocation
    pub next_allocation_id: u64,
    pub claims: Cid, // HAMT[Address]HAMT[ClaimID]Claim
}

impl State {
    pub fn new<BS: Blockstore>(store: &BS, root_key: Address) -> anyhow::Result<State> {
        let empty_map = make_empty_map::<_, ()>(store, HAMT_BIT_WIDTH)
            .flush()
            .map_err(|e| anyhow::anyhow!("Failed to create empty map: {}", e))?;

        let empty_mapmap = MapMap::new(store, HAMT_BIT_WIDTH, HAMT_BIT_WIDTH).root().map_err(|e| anyhow::anyhow!("Failed to create empty multi map: {}", e))?;

        Ok(State {
            root_key,
            verifiers: empty_map,
            verified_clients: empty_map,
            remove_data_cap_proposal_ids: empty_map,
            allocations: empty_mapmap,
            next_allocation_id: 0,
            claims: empty_mapmap,
        })
    }
}
#[derive(Serialize_tuple, Deserialize_tuple, Clone, Debug, PartialEq)]
pub struct Claim {
    pub provider: Address,
    pub client: Address,
    pub data: Cid,
    pub size: u64,
    // The min period which the provider must commit to storing data
    pub term_min: u64,
    // The max period for which provider can earn QA-power for the data
    pub term_max: u64,
    pub term_start: ChainEpoch,
    pub sector: SectorID, 
}

#[derive(Serialize_tuple, Deserialize_tuple, Clone, Debug)]
pub struct Allocation {
    pub client: Address,
    pub provider: Address,
    pub data: Cid,
    pub size: u64,
    pub term_min: u64,
    pub term_max: u64,
    pub expiration: u64,
}

impl Cbor for State {}
