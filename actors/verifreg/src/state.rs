// Copyright 2019-2022 ChainSafe Systems
// SPDX-License-Identifier: Apache-2.0, MIT

use cid::Cid;
use fvm_ipld_blockstore::Blockstore;
use fvm_ipld_encoding::tuple::*;
use fvm_shared::address::Address;
use fvm_shared::bigint::bigint_ser::BigIntDe;
use fvm_shared::clock::ChainEpoch;
use fvm_shared::error::ExitCode;
use fvm_shared::piece::PaddedPieceSize;
use fvm_shared::sector::SectorNumber;
use fvm_shared::{ActorID, HAMT_BIT_WIDTH};

use fil_actors_runtime::{
    ActorError, AsActorError, Config, DEFAULT_HAMT_CONFIG, Map2, MapMap, actor_error,
};

use crate::{AddrPairKey, AllocationID, ClaimID};
use crate::{DataCap, RemoveDataCapProposalID};

pub type DataCapMap<BS> = Map2<BS, Address, BigIntDe>;
pub const DATACAP_MAP_CONFIG: Config = DEFAULT_HAMT_CONFIG;

pub type RemoveDataCapProposalMap<BS> = Map2<BS, AddrPairKey, RemoveDataCapProposalID>;
pub const REMOVE_DATACAP_PROPOSALS_CONFIG: Config = DEFAULT_HAMT_CONFIG;

#[derive(Serialize_tuple, Deserialize_tuple, Debug, Clone)]
pub struct State {
    pub root_key: Address,
    // Maps verifier addresses to data cap minting allowance (in bytes).
    pub verifiers: Cid, // HAMT[Address]DataCap
    pub remove_data_cap_proposal_ids: Cid,
    // Maps client IDs to allocations made by that client.
    pub allocations: Cid, // HAMT[ActorID]HAMT[AllocationID]Allocation
    // Next allocation identifier to use.
    // The value 0 is reserved to mean "no allocation".
    pub next_allocation_id: u64,
    // Maps provider IDs to allocations claimed by that provider.
    pub claims: Cid, // HAMT[ActorID]HAMT[ClaimID]Claim
}

impl State {
    pub fn new<BS: Blockstore>(store: &BS, root_key: Address) -> Result<State, ActorError> {
        let empty_dcap = DataCapMap::empty(store, DATACAP_MAP_CONFIG, "empty").flush()?;
        let empty_allocs_claims =
            MapMap::<_, (), ActorID, u64>::new(store, HAMT_BIT_WIDTH, HAMT_BIT_WIDTH)
                .flush()
                .map_err(|e| {
                    actor_error!(illegal_state, "failed to create empty multi map: {}", e)
                })?;

        Ok(State {
            root_key,
            verifiers: empty_dcap,
            remove_data_cap_proposal_ids: empty_dcap,
            allocations: empty_allocs_claims,
            next_allocation_id: 1,
            claims: empty_allocs_claims,
        })
    }

    // Adds a verifier and cap, overwriting any existing cap for that verifier.
    pub fn put_verifier(
        &mut self,
        store: &impl Blockstore,
        verifier: &Address,
        cap: &DataCap,
    ) -> Result<(), ActorError> {
        let mut verifiers = self.load_verifiers(store)?;
        verifiers.set(verifier, BigIntDe(cap.clone()))?;
        self.verifiers = verifiers.flush()?;
        Ok(())
    }

    pub fn remove_verifier(
        &mut self,
        store: &impl Blockstore,
        verifier: &Address,
    ) -> Result<(), ActorError> {
        let mut verifiers = self.load_verifiers(store)?;
        verifiers
            .delete(verifier)?
            .context_code(ExitCode::USR_ILLEGAL_ARGUMENT, "verifier not found")?;
        self.verifiers = verifiers.flush()?;
        Ok(())
    }

    pub fn get_verifier_cap(
        &self,
        store: &impl Blockstore,
        verifier: &Address,
    ) -> Result<Option<DataCap>, ActorError> {
        let verifiers = self.load_verifiers(store)?;
        let allowance = verifiers.get(verifier)?;
        Ok(allowance.map(|a| a.clone().0))
    }

    pub fn load_verifiers<BS: Blockstore>(&self, store: BS) -> Result<DataCapMap<BS>, ActorError> {
        DataCapMap::load(store, &self.verifiers, DATACAP_MAP_CONFIG, "verifiers")
    }

    pub fn load_allocs<'a, BS: Blockstore>(
        &self,
        store: &'a BS,
    ) -> Result<MapMap<'a, BS, Allocation, ActorID, AllocationID>, ActorError> {
        MapMap::<BS, Allocation, ActorID, AllocationID>::from_root(
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
    ) -> Result<MapMap<'a, BS, Claim, ActorID, ClaimID>, ActorError> {
        MapMap::<BS, Claim, ActorID, ClaimID>::from_root(
            store,
            &self.claims,
            HAMT_BIT_WIDTH,
            HAMT_BIT_WIDTH,
        )
        .context_code(ExitCode::USR_ILLEGAL_STATE, "failed to load claims table")
    }

    pub fn save_claims<BS: Blockstore>(
        &mut self,
        claims: &mut MapMap<'_, BS, Claim, ActorID, ClaimID>,
    ) -> Result<(), ActorError> {
        self.claims = claims
            .flush()
            .context_code(ExitCode::USR_ILLEGAL_STATE, "failed to flush claims table")?;
        Ok(())
    }
}
#[derive(Serialize_tuple, Deserialize_tuple, Clone, Debug, PartialEq, Eq)]
pub struct Claim {
    // The provider storing the data (from allocation).
    pub provider: ActorID,
    // The client which allocated the DataCap (from allocation).
    pub client: ActorID,
    // Identifier of the data committed (from allocation).
    pub data: Cid,
    // The (padded) size of data (from allocation).
    pub size: PaddedPieceSize,
    // The min period after term_start which the provider must commit to storing data
    pub term_min: ChainEpoch,
    // The max period after term_start for which provider can earn QA-power for the data
    pub term_max: ChainEpoch,
    // The epoch at which the (first range of the) piece was committed.
    pub term_start: ChainEpoch,
    // ID of the provider's sector in which the data is committed.
    pub sector: SectorNumber,
}

#[derive(Serialize_tuple, Deserialize_tuple, Clone, Debug, PartialEq, Eq)]
pub struct Allocation {
    // The verified client which allocated the DataCap.
    pub client: ActorID,
    // The provider (miner actor) which may claim the allocation.
    pub provider: ActorID,
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

pub fn get_claim<'a, BS>(
    claims: &'a mut MapMap<BS, Claim, ActorID, ClaimID>,
    provider: ActorID,
    id: ClaimID,
) -> Result<Option<&'a Claim>, ActorError>
where
    BS: Blockstore,
{
    claims
        .get(provider, id)
        .context_code(ExitCode::USR_ILLEGAL_STATE, "HAMT lookup failure getting claim")
}
