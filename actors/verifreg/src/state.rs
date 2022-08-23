// Copyright 2019-2022 ChainSafe Systems
// SPDX-License-Identifier: Apache-2.0, MIT

use cid::Cid;
use fvm_ipld_blockstore::Blockstore;
use fvm_ipld_encoding::tuple::*;
use fvm_ipld_encoding::Cbor;
use fvm_shared::address::Address;
use fvm_shared::bigint::bigint_ser::BigIntDe;
use fvm_shared::clock::ChainEpoch;
use fvm_shared::error::ExitCode;
use fvm_shared::piece::PaddedPieceSize;
use fvm_shared::sector::SectorID;
use fvm_shared::HAMT_BIT_WIDTH;

use fil_actors_runtime::{
    actor_error, make_empty_map, make_map_with_root_and_bitwidth, ActorError, AsActorError, MapMap,
};

use crate::DataCap;
use crate::{AllocationID, ClaimID};

#[derive(Serialize_tuple, Deserialize_tuple)]
pub struct State {
    pub root_key: Address,
    // Address of the data cap token actor
    pub token: Address,
    // Maps verifier addresses to data cap minting allowance (in bytes).
    pub verifiers: Cid,
    pub remove_data_cap_proposal_ids: Cid,
    pub allocations: Cid, // HAMT[Address]HAMT[AllocationID]Allocation
    pub next_allocation_id: u64,
    pub claims: Cid, // HAMT[Address]HAMT[ClaimID]Claim
}

impl State {
    pub fn new<BS: Blockstore>(
        store: &BS,
        root_key: Address,
        token: Address,
    ) -> Result<State, ActorError> {
        let empty_map = make_empty_map::<_, ()>(store, HAMT_BIT_WIDTH)
            .flush()
            .map_err(|e| actor_error!(illegal_state, "failed to create empty map: {}", e))?;

        let empty_mapmap =
            MapMap::<_, (), Address, AllocationID>::new(store, HAMT_BIT_WIDTH, HAMT_BIT_WIDTH)
                .flush()
                .map_err(|e| {
                    actor_error!(illegal_state, "Failed to create empty multi map: {}", e)
                })?;

        Ok(State {
            root_key,
            token,
            verifiers: empty_map,
            remove_data_cap_proposal_ids: empty_map,
            allocations: empty_mapmap,
            next_allocation_id: 1,
            claims: empty_mapmap,
        })
    }

    // Adds a verifier and cap, overwriting any existing cap for that verifier.
    pub fn put_verifier(
        &mut self,
        store: &impl Blockstore,
        verifier: &Address,
        cap: &DataCap,
    ) -> Result<(), ActorError> {
        let mut verifiers =
            make_map_with_root_and_bitwidth::<_, BigIntDe>(&self.verifiers, store, HAMT_BIT_WIDTH)
                .context_code(ExitCode::USR_ILLEGAL_STATE, "failed to load verifiers")?;
        // .context("failed to load verifiers")?;
        verifiers
            .set(verifier.to_bytes().into(), BigIntDe(cap.clone()))
            .context_code(ExitCode::USR_ILLEGAL_STATE, "failed to set verifier")?;
        self.verifiers = verifiers
            .flush()
            .context_code(ExitCode::USR_ILLEGAL_STATE, "failed to flush verifiers")?;
        Ok(())
    }

    pub fn remove_verifier(
        &mut self,
        store: &impl Blockstore,
        verifier: &Address,
    ) -> Result<(), ActorError> {
        let mut verifiers =
            make_map_with_root_and_bitwidth::<_, BigIntDe>(&self.verifiers, store, HAMT_BIT_WIDTH)
                .context_code(ExitCode::USR_ILLEGAL_STATE, "failed to load verifiers")?;

        verifiers
            .delete(&verifier.to_bytes())
            .context_code(ExitCode::USR_ILLEGAL_STATE, "failed to remove verifier")?
            .context_code(ExitCode::USR_ILLEGAL_ARGUMENT, "verifier not found")?;

        self.verifiers = verifiers
            .flush()
            .context_code(ExitCode::USR_ILLEGAL_STATE, "failed to flush verifiers")?;
        Ok(())
    }

    pub fn get_verifier_cap(
        &self,
        store: &impl Blockstore,
        verifier: &Address,
    ) -> Result<Option<DataCap>, ActorError> {
        let verifiers =
            make_map_with_root_and_bitwidth::<_, BigIntDe>(&self.verifiers, store, HAMT_BIT_WIDTH)
                .context_code(ExitCode::USR_ILLEGAL_STATE, "failed to load verifiers")?;
        let allowance = verifiers
            .get(&verifier.to_bytes())
            .context_code(ExitCode::USR_ILLEGAL_STATE, "failed to get verifier")?;
        Ok(allowance.map(|a| a.0.clone() as DataCap)) // TODO: can I avoid the clone?
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
