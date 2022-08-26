// Copyright 2019-2022 ChainSafe Systems
// SPDX-License-Identifier: Apache-2.0, MIT

use cid::Cid;
use fil_actors_runtime::{make_empty_map, MapMap, AsActorError, ActorError};
use fvm_ipld_blockstore::Blockstore;
use fvm_ipld_encoding::tuple::*;
use fvm_ipld_encoding::Cbor;
use fvm_shared::address::Address;
use fvm_shared::HAMT_BIT_WIDTH;
use fvm_shared::error::ExitCode;
use fvm_shared::sector::SectorID;
use fvm_shared::clock::{ChainEpoch};
use fvm_shared::piece::PaddedPieceSize;
use crate::{AllocationID, ClaimID, DataCap};

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

        let empty_mapmap = MapMap::<_, (), Address, AllocationID>::new(store, HAMT_BIT_WIDTH, HAMT_BIT_WIDTH).flush().map_err(|e| anyhow::anyhow!("Failed to create empty multi map: {}", e))?;

        Ok(State {
            root_key,
            verifiers: empty_map,
            verified_clients: empty_map,
            remove_data_cap_proposal_ids: empty_map,
            allocations: empty_mapmap,
            next_allocation_id: 1,
            claims: empty_mapmap,
        })
    }
    pub fn load_allocs<'a, BS: Blockstore>(&self, store: &'a BS)  -> Result<MapMap::<'a, BS, Allocation, Address, AllocationID>, ActorError> {
        MapMap::<BS, Allocation, Address, AllocationID>::from_root(store, &self.allocations, HAMT_BIT_WIDTH, HAMT_BIT_WIDTH).context_code(ExitCode::USR_ILLEGAL_STATE, "failed to load allocations table")
    }

    pub fn load_claims<'a, BS: Blockstore>(&self, store: &'a BS)  -> Result<MapMap::<'a, BS, Claim, Address, ClaimID>, ActorError> {
        MapMap::<BS, Claim, Address, ClaimID>::from_root(store, &self.allocations, HAMT_BIT_WIDTH, HAMT_BIT_WIDTH).context_code(ExitCode::USR_ILLEGAL_STATE, "failed to load claims table")
    }
}
#[derive(Serialize_tuple, Deserialize_tuple, Clone, Debug, PartialEq)]
pub struct Claim {
    pub provider: Address,
    pub client: Address,
    pub data: Cid,
    pub size: PaddedPieceSize,
    // The min period which the provider must commit to storing data
    pub term_min: ChainEpoch,
    // The max period for which provider can earn QA-power for the data
    pub term_max: ChainEpoch,
    pub term_start: ChainEpoch,
    pub sector: SectorID, 
}

#[derive(Serialize_tuple, Deserialize_tuple, Clone, Debug, PartialEq)]
pub struct Allocation {
    pub client: Address,
    pub provider: Address,
    pub data: Cid,
    pub size: PaddedPieceSize,
    pub term_min: ChainEpoch,
    pub term_max: ChainEpoch,
    pub expiration: ChainEpoch,
}

impl Cbor for State {}

