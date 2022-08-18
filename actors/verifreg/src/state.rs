// Copyright 2019-2022 ChainSafe Systems
// SPDX-License-Identifier: Apache-2.0, MIT

use crate::{AllocationID, ClaimID};
use cid::Cid;
use fil_actors_runtime::{make_empty_map, ActorError, AsActorError, MapMap};
use fvm_ipld_blockstore::Blockstore;
use fvm_ipld_encoding::tuple::*;
use fvm_ipld_encoding::Cbor;
use fvm_shared::address::Address;
use fvm_shared::clock::ChainEpoch;
use fvm_shared::error::ExitCode;
use fvm_shared::piece::PaddedPieceSize;
use fvm_shared::sector::SectorID;
use fvm_shared::HAMT_BIT_WIDTH;

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

        let empty_mapmap =
            MapMap::<_, (), Address, AllocationID>::new(store, HAMT_BIT_WIDTH, HAMT_BIT_WIDTH)
                .flush()
                .map_err(|e| anyhow::anyhow!("Failed to create empty multi map: {}", e))?;

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
    pub fn load_allocs<'a, BS: Blockstore>(
        &self,
        store: &'a BS,
    ) -> Result<MapMap<'a, BS, Allocation, Address, AllocationID>, ActorError> {
        MapMap::<BS, Allocation, Address, AllocationID>::from_root(
            store,
            &self.allocations,
            HAMT_BIT_WIDTH,
            HAMT_BIT_WIDTH,
        )
        .context_code(ExitCode::USR_ILLEGAL_STATE, "failed to load allocations table")
    }

    pub fn load_claims<'a, BS: Blockstore>(
        &self,
        store: &'a BS,
    ) -> Result<MapMap<'a, BS, Claim, Address, ClaimID>, ActorError> {
        MapMap::<BS, Claim, Address, ClaimID>::from_root(
            store,
            &self.allocations,
            HAMT_BIT_WIDTH,
            HAMT_BIT_WIDTH,
        )
        .context_code(ExitCode::USR_ILLEGAL_STATE, "failed to load claims table")
    }
}
#[derive(Serialize_tuple, Deserialize_tuple, Clone, Debug, PartialEq)]
pub struct Claim {
    // The provider storing the data (from allocation).
    pub provider: Address,
    // The client which allocated the DataCap (from allocation).
    pub client: Address,
    // Identifier of the data committed (from allocation).
    pub data: Cid,
    // The (padded) size of data (from allocation).
    pub size: PaddedPieceSize,
    // The min period which the provider must commit to storing data
    pub term_min: ChainEpoch,
    // The max period for which provider can earn QA-power for the data
    pub term_max: ChainEpoch,
    // The epoch at which the (first range of the) piece was committed.
    pub term_start: ChainEpoch,
    // ID of the provider's sector in which the data is committed.
    pub sector: SectorID,
}

#[derive(Serialize_tuple, Deserialize_tuple, Clone, Debug, PartialEq, Eq)]
pub struct Allocation {
    // The verified client which allocated the DataCap.
    pub client: Address,
    // The provider (miner actor) which may claim the allocation.
    pub provider: Address,
    // Identifier of the data to be committed.
    pub data: Cid,
    // The (padded) size of data.
    pub size: PaddedPieceSize,
    // The minimum duration which the provider must commit to storing the piece to avoid
    // early-termination penalties (epochs).
    pub term_min: ChainEpoch,
    // The maximum period for which a provider can earn quality-adjusted power
    // for the piece (epochs).
    pub term_max: ChainEpoch,
    // The latest epoch by which a provider must commit data before the allocation expires.
    pub expiration: ChainEpoch,
}

impl Cbor for State {}
