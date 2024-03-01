// Copyright 2019-2022 ChainSafe Systems
// SPDX-License-Identifier: Apache-2.0, MIT

use std::cmp::{max, min};
use std::collections::BTreeMap;

use cid::Cid;
use fvm_ipld_blockstore::Blockstore;
use fvm_ipld_encoding::tuple::*;
use fvm_shared::address::Address;
use fvm_shared::clock::{ChainEpoch, EPOCH_UNDEFINED};
use fvm_shared::deal::DealID;
use fvm_shared::econ::TokenAmount;
use fvm_shared::error::ExitCode;
use fvm_shared::sector::SectorNumber;
use fvm_shared::{ActorID, HAMT_BIT_WIDTH};
use num_traits::Zero;

use fil_actors_runtime::{
    actor_error, ActorContext, ActorError, Array, AsActorError, Config, Map2, Set, SetMultimap,
    SetMultimapConfig, DEFAULT_HAMT_CONFIG,
};

use crate::balance_table::BalanceTable;
use crate::ext::verifreg::AllocationID;

use super::policy::*;
use super::types::*;
use super::{DealProposal, DealState, EX_DEAL_EXPIRED};

pub enum Reason {
    ClientCollateral,
    ClientStorageFee,
    ProviderCollateral,
}

/// Market actor state
#[derive(Clone, Default, Serialize_tuple, Deserialize_tuple, Debug)]
pub struct State {
    /// Proposals are deals that have been proposed and not yet cleaned up after expiry or termination.
    /// Array<DealID, DealProposal>
    pub proposals: Cid,

    // States contains state for deals that have been activated and not yet cleaned up after expiry or termination.
    // After expiration, the state exists until the proposal is cleaned up too.
    // Invariant: keys(States) ⊆ keys(Proposals).
    /// Array<DealID, DealState>
    pub states: Cid,

    /// PendingProposals tracks dealProposals that have not yet reached their deal start date.
    /// We track them here to ensure that miners can't publish the same deal proposal twice
    /// Set<CID>
    pub pending_proposals: Cid,

    /// Total amount held in escrow, indexed by actor address (including both locked and unlocked amounts).
    pub escrow_table: Cid,

    /// Amount locked, indexed by actor address.
    /// Note: the amounts in this table do not affect the overall amount in escrow:
    /// only the _portion_ of the total escrow amount that is locked.
    pub locked_table: Cid,

    /// Deal id state sequential incrementer
    pub next_id: DealID,

    /// Metadata cached for efficient iteration over deals.
    /// SetMultimap<Address>
    pub deal_ops_by_epoch: Cid,
    pub last_cron: ChainEpoch,

    /// Total Client Collateral that is locked -> unlocked when deal is terminated
    pub total_client_locked_collateral: TokenAmount,
    /// Total Provider Collateral that is locked -> unlocked when deal is terminated
    pub total_provider_locked_collateral: TokenAmount,
    /// Total storage fee that is locked in escrow -> unlocked when payments are made
    pub total_client_storage_fee: TokenAmount,

    /// Verified registry allocation IDs for deals that are not yet activated.
    // HAMT[DealID]AllocationID
    pub pending_deal_allocation_ids: Cid,

    /// Maps providers to their sector IDs to deal IDs.
    /// This supports finding affected deals when a sector is terminated early
    /// or has data replaced.
    /// Grouping by provider limits the cost of operations in the expected use case
    /// of multiple sectors all belonging to the same provider.
    /// HAMT[ActorID]HAMT[SectorNumber][]DealID
    pub provider_sectors: Cid,
}

pub type PendingProposalsSet<BS> = Set<BS, Cid>;
pub const PENDING_PROPOSALS_CONFIG: Config = DEFAULT_HAMT_CONFIG;

pub type DealOpsByEpoch<BS> = SetMultimap<BS, ChainEpoch, DealID>;
pub const DEAL_OPS_BY_EPOCH_CONFIG: SetMultimapConfig =
    SetMultimapConfig { outer: DEFAULT_HAMT_CONFIG, inner: DEFAULT_HAMT_CONFIG };

pub type PendingDealAllocationsMap<BS> = Map2<BS, DealID, AllocationID>;
pub const PENDING_ALLOCATIONS_CONFIG: Config =
    Config { bit_width: HAMT_BIT_WIDTH, ..DEFAULT_HAMT_CONFIG };

pub type ProviderSectorsMap<BS> = Map2<BS, ActorID, Cid>;
pub const PROVIDER_SECTORS_CONFIG: Config =
    Config { bit_width: HAMT_BIT_WIDTH, ..DEFAULT_HAMT_CONFIG };

pub type SectorDealsMap<BS> = Map2<BS, SectorNumber, Vec<DealID>>;
pub const SECTOR_DEALS_CONFIG: Config = Config { bit_width: HAMT_BIT_WIDTH, ..DEFAULT_HAMT_CONFIG };

impl State {
    pub fn new<BS: Blockstore>(store: &BS) -> Result<Self, ActorError> {
        let empty_proposals_array =
            Array::<(), BS>::new_with_bit_width(store, PROPOSALS_AMT_BITWIDTH)
                .flush()
                .context_code(
                    ExitCode::USR_ILLEGAL_STATE,
                    "failed to create empty proposals array",
                )?;

        let empty_states_array = Array::<(), BS>::new_with_bit_width(store, STATES_AMT_BITWIDTH)
            .flush()
            .context_code(ExitCode::USR_ILLEGAL_STATE, "failed to create empty states array")?;

        let empty_pending_proposals =
            PendingProposalsSet::empty(store, PENDING_PROPOSALS_CONFIG, "pending proposals")
                .flush()?;
        let empty_balance_table = BalanceTable::new(store, "balance table").root()?;
        let empty_deal_ops =
            DealOpsByEpoch::empty(store, DEAL_OPS_BY_EPOCH_CONFIG, "deal ops").flush()?;

        let empty_pending_deal_allocation_map = PendingDealAllocationsMap::empty(
            store,
            PENDING_ALLOCATIONS_CONFIG,
            "pending deal allocations",
        )
        .flush()?;

        let empty_sector_deals_hamt =
            ProviderSectorsMap::empty(store, PROVIDER_SECTORS_CONFIG, "sector deals").flush()?;

        Ok(Self {
            proposals: empty_proposals_array,
            states: empty_states_array,
            pending_proposals: empty_pending_proposals,
            escrow_table: empty_balance_table,
            locked_table: empty_balance_table,
            next_id: 0,
            deal_ops_by_epoch: empty_deal_ops,
            last_cron: EPOCH_UNDEFINED,

            total_client_locked_collateral: TokenAmount::default(),
            total_provider_locked_collateral: TokenAmount::default(),
            total_client_storage_fee: TokenAmount::default(),
            pending_deal_allocation_ids: empty_pending_deal_allocation_map,
            provider_sectors: empty_sector_deals_hamt,
        })
    }

    pub fn get_total_locked(&self) -> TokenAmount {
        &self.total_client_locked_collateral
            + &self.total_provider_locked_collateral
            + &self.total_client_storage_fee
    }

    pub fn load_deal_states<'bs, BS>(
        &self,
        store: &'bs BS,
    ) -> Result<DealMetaArray<'bs, BS>, ActorError>
    where
        BS: Blockstore,
    {
        DealMetaArray::load(&self.states, store)
            .context_code(ExitCode::USR_ILLEGAL_STATE, "failed to load deal state array")
    }

    fn save_deal_states<BS>(&mut self, states: &mut DealMetaArray<BS>) -> Result<(), ActorError>
    where
        BS: Blockstore,
    {
        self.states = states
            .flush()
            .context_code(ExitCode::USR_ILLEGAL_STATE, "failed to flush deal states")?;
        Ok(())
    }

    pub fn find_deal_state<BS>(
        &self,
        store: &BS,
        deal_id: DealID,
    ) -> Result<Option<DealState>, ActorError>
    where
        BS: Blockstore,
    {
        let states = self.load_deal_states(store)?;
        find_deal_state(&states, deal_id)
    }

    pub fn put_deal_states<BS>(
        &mut self,
        store: &BS,
        new_deal_states: &[(DealID, DealState)],
    ) -> Result<(), ActorError>
    where
        BS: Blockstore,
    {
        let mut states = self.load_deal_states(store)?;
        new_deal_states.iter().try_for_each(|(id, deal_state)| -> Result<(), ActorError> {
            states
                .set(*id, *deal_state)
                .context_code(ExitCode::USR_ILLEGAL_STATE, "failed to set deal state")?;
            Ok(())
        })?;
        self.save_deal_states(&mut states)
    }

    pub fn remove_deal_state<BS>(
        &mut self,
        store: &BS,
        deal_id: DealID,
    ) -> Result<Option<DealState>, ActorError>
    where
        BS: Blockstore,
    {
        let mut states = self.load_deal_states(store)?;
        let removed = states
            .delete(deal_id)
            .context_code(ExitCode::USR_ILLEGAL_STATE, "failed to delete deal state")?;
        self.save_deal_states(&mut states)?;
        Ok(removed)
    }

    pub fn load_proposals<'bs, BS>(&self, store: &'bs BS) -> Result<DealArray<'bs, BS>, ActorError>
    where
        BS: Blockstore,
    {
        DealArray::load(&self.proposals, store)
            .context_code(ExitCode::USR_ILLEGAL_STATE, "failed to load deal proposal array")
    }

    pub fn get_proposal<BS: Blockstore>(
        &self,
        store: &BS,
        id: DealID,
    ) -> Result<DealProposal, ActorError> {
        get_proposal(&self.load_proposals(store)?, id, self.next_id)
    }

    pub fn find_proposal<BS>(
        &self,
        store: &BS,
        deal_id: DealID,
    ) -> Result<Option<DealProposal>, ActorError>
    where
        BS: Blockstore,
    {
        find_proposal(&self.load_proposals(store)?, deal_id)
    }

    pub fn remove_proposal<BS>(
        &mut self,
        store: &BS,
        deal_id: DealID,
    ) -> Result<Option<DealProposal>, ActorError>
    where
        BS: Blockstore,
    {
        let mut deal_proposals = DealArray::load(&self.proposals, store)
            .context_code(ExitCode::USR_ILLEGAL_STATE, "failed to load deal proposal array")?;

        let proposal = deal_proposals
            .delete(deal_id)
            .with_context_code(ExitCode::USR_ILLEGAL_STATE, || {
                format!("no such deal proposal {}", deal_id)
            })?;

        self.proposals = deal_proposals
            .flush()
            .context_code(ExitCode::USR_ILLEGAL_STATE, "failed to flush deal proposals")?;

        Ok(proposal)
    }

    pub fn put_deal_proposals<BS>(
        &mut self,
        store: &BS,
        new_deal_proposals: &[(DealID, DealProposal)],
    ) -> Result<(), ActorError>
    where
        BS: Blockstore,
    {
        let mut deal_proposals = DealArray::load(&self.proposals, store)
            .context_code(ExitCode::USR_ILLEGAL_STATE, "failed to load deal proposal array")?;

        new_deal_proposals.iter().try_for_each(|(id, proposal)| -> Result<(), ActorError> {
            deal_proposals
                .set(*id, proposal.clone())
                .context_code(ExitCode::USR_ILLEGAL_STATE, "failed to set deal proposal")?;
            Ok(())
        })?;

        self.proposals = deal_proposals
            .flush()
            .context_code(ExitCode::USR_ILLEGAL_STATE, "failed to flush deal proposals")?;

        Ok(())
    }

    pub fn load_pending_deal_allocation_ids<BS>(
        &mut self,
        store: BS,
    ) -> Result<PendingDealAllocationsMap<BS>, ActorError>
    where
        BS: Blockstore,
    {
        PendingDealAllocationsMap::load(
            store,
            &self.pending_deal_allocation_ids,
            PENDING_ALLOCATIONS_CONFIG,
            "pending deal allocations",
        )
    }

    pub fn save_pending_deal_allocation_ids<BS>(
        &mut self,
        pending_deal_allocation_ids: &mut PendingDealAllocationsMap<BS>,
    ) -> Result<(), ActorError>
    where
        BS: Blockstore,
    {
        self.pending_deal_allocation_ids = pending_deal_allocation_ids.flush()?;
        Ok(())
    }

    pub fn put_pending_deal_allocation_ids<BS>(
        &mut self,
        store: &BS,
        new_pending_deal_allocation_ids: &[(DealID, AllocationID)],
    ) -> Result<(), ActorError>
    where
        BS: Blockstore,
    {
        let mut pending_deal_allocation_ids = self.load_pending_deal_allocation_ids(store)?;
        new_pending_deal_allocation_ids.iter().try_for_each(
            |(deal_id, allocation_id)| -> Result<(), ActorError> {
                pending_deal_allocation_ids.set(deal_id, *allocation_id)?;
                Ok(())
            },
        )?;
        self.save_pending_deal_allocation_ids(&mut pending_deal_allocation_ids)?;
        Ok(())
    }

    pub fn get_pending_deal_allocation_ids<BS>(
        &mut self,
        store: &BS,
        deal_id_keys: &[DealID],
    ) -> Result<Vec<AllocationID>, ActorError>
    where
        BS: Blockstore,
    {
        let pending_deal_allocation_ids = self.load_pending_deal_allocation_ids(store)?;

        let mut allocation_ids: Vec<AllocationID> = vec![];
        deal_id_keys.iter().try_for_each(|deal_id| -> Result<(), ActorError> {
            let allocation_id = pending_deal_allocation_ids.get(&deal_id.clone())?;
            allocation_ids.push(
                *allocation_id.ok_or(ActorError::not_found("no such deal proposal".to_string()))?,
            );
            Ok(())
        })?;

        Ok(allocation_ids)
    }

    pub fn remove_pending_deal_allocation_id<BS>(
        &mut self,
        store: &BS,
        deal_id: DealID,
    ) -> Result<Option<AllocationID>, ActorError>
    where
        BS: Blockstore,
    {
        let mut pending_deal_allocation_ids = self.load_pending_deal_allocation_ids(store)?;
        let maybe_alloc_id = pending_deal_allocation_ids.delete(&deal_id)?;
        self.save_pending_deal_allocation_ids(&mut pending_deal_allocation_ids)?;
        Ok(maybe_alloc_id)
    }

    pub fn load_deal_ops<BS>(
        &self,
        store: BS,
    ) -> Result<SetMultimap<BS, ChainEpoch, DealID>, ActorError>
    where
        BS: Blockstore,
    {
        DealOpsByEpoch::load(store, &self.deal_ops_by_epoch, DEAL_OPS_BY_EPOCH_CONFIG, "deal ops")
    }

    pub fn put_deals_by_epoch<BS>(
        &mut self,
        store: &BS,
        new_deals_by_epoch: &[(ChainEpoch, DealID)],
    ) -> Result<(), ActorError>
    where
        BS: Blockstore,
    {
        let mut deals_by_epoch = self.load_deal_ops(store)?;
        new_deals_by_epoch.iter().try_for_each(|(epoch, id)| -> Result<(), ActorError> {
            deals_by_epoch.put(epoch, *id)?;
            Ok(())
        })?;

        self.deal_ops_by_epoch = deals_by_epoch.flush()?;
        Ok(())
    }

    pub fn put_batch_deals_by_epoch<BS>(
        &mut self,
        store: &BS,
        new_deals_by_epoch: &BTreeMap<ChainEpoch, Vec<DealID>>,
    ) -> Result<(), ActorError>
    where
        BS: Blockstore,
    {
        let mut deals_by_epoch = self.load_deal_ops(store)?;
        new_deals_by_epoch.iter().try_for_each(|(epoch, deals)| -> Result<(), ActorError> {
            deals_by_epoch.put_many(epoch, deals)?;
            Ok(())
        })?;

        self.deal_ops_by_epoch = deals_by_epoch.flush()?;
        Ok(())
    }

    pub fn get_deals_for_epoch<BS>(
        &self,
        store: &BS,
        key: ChainEpoch,
    ) -> Result<Vec<DealID>, ActorError>
    where
        BS: Blockstore,
    {
        let mut deal_ids = Vec::new();
        let deals_by_epoch = self.load_deal_ops(store)?;
        deals_by_epoch.for_each_in(&key, |deal_id| {
            deal_ids.push(deal_id);
            Ok(())
        })?;

        Ok(deal_ids)
    }

    pub fn remove_deals_by_epoch<BS>(
        &mut self,
        store: &BS,
        epochs_to_remove: &[ChainEpoch],
    ) -> Result<(), ActorError>
    where
        BS: Blockstore,
    {
        let mut deals_by_epoch = self.load_deal_ops(store)?;
        epochs_to_remove.iter().try_for_each(|epoch| -> Result<(), ActorError> {
            deals_by_epoch.remove_all(epoch)?;
            Ok(())
        })?;

        self.deal_ops_by_epoch = deals_by_epoch.flush()?;
        Ok(())
    }

    pub fn add_balance_to_escrow_table<BS>(
        &mut self,
        store: &BS,
        addr: &Address,
        amount: &TokenAmount,
    ) -> Result<(), ActorError>
    where
        BS: Blockstore,
    {
        let mut escrow_table = BalanceTable::from_root(store, &self.escrow_table, "escrow table")?;
        escrow_table.add(addr, amount)?;
        self.escrow_table = escrow_table.root()?;
        Ok(())
    }

    pub fn withdraw_balance_from_escrow_table<BS>(
        &mut self,
        store: &BS,
        addr: &Address,
        amount: &TokenAmount,
    ) -> Result<TokenAmount, ActorError>
    where
        BS: Blockstore,
    {
        let mut escrow_table = BalanceTable::from_root(store, &self.escrow_table, "escrow table")?;
        let locked_table = BalanceTable::from_root(store, &self.locked_table, "locked table")?;

        let min_balance = locked_table.get(addr)?;
        let ex = escrow_table.subtract_with_minimum(addr, amount, &min_balance)?;

        self.escrow_table = escrow_table.root()?;
        Ok(ex)
    }

    pub fn load_pending_deals<BS>(&self, store: BS) -> Result<PendingProposalsSet<BS>, ActorError>
    where
        BS: Blockstore,
    {
        PendingProposalsSet::load(
            store,
            &self.pending_proposals,
            PENDING_PROPOSALS_CONFIG,
            "pending proposals",
        )
    }

    fn save_pending_deals<BS>(
        &mut self,
        pending_deals: &mut PendingProposalsSet<BS>,
    ) -> Result<(), ActorError>
    where
        BS: Blockstore,
    {
        self.pending_proposals = pending_deals.flush()?;
        Ok(())
    }

    pub fn has_pending_deal<BS>(&self, store: &BS, key: &Cid) -> Result<bool, ActorError>
    where
        BS: Blockstore,
    {
        let pending_deals = self.load_pending_deals(store)?;
        pending_deals.has(key)
    }

    pub fn put_pending_deals<BS>(
        &mut self,
        store: &BS,
        new_pending_deals: &[Cid],
    ) -> Result<(), ActorError>
    where
        BS: Blockstore,
    {
        let mut pending_deals = self.load_pending_deals(store)?;
        new_pending_deals.iter().try_for_each(|key: &Cid| -> Result<(), ActorError> {
            pending_deals.put(key)?;
            Ok(())
        })?;

        self.save_pending_deals(&mut pending_deals)
    }

    pub fn remove_pending_deal<BS>(
        &mut self,
        store: &BS,
        pending_deal_key: Cid,
    ) -> Result<Option<()>, ActorError>
    where
        BS: Blockstore,
    {
        let mut pending_deals = self.load_pending_deals(store)?;
        let removed = pending_deals.delete(&pending_deal_key)?;

        self.save_pending_deals(&mut pending_deals)?;
        Ok(removed)
    }

    ////////////////////////////////////////////////////////////////////////////////
    // Provider sector/deal operations
    ////////////////////////////////////////////////////////////////////////////////

    // Stores deal IDs associated with sectors for a provider.
    // Deal IDs are added to any already stored for the provider and sector.
    // Returns the root cid of the sector deals map.
    pub fn put_sector_deal_ids(
        &mut self,
        store: &impl Blockstore,
        provider: ActorID,
        sector_deal_ids: &[(SectorNumber, Vec<DealID>)],
    ) -> Result<(), ActorError> {
        let mut provider_sectors = self.load_provider_sectors(store)?;
        let mut sector_deals = load_provider_sector_deals(store, &provider_sectors, provider)?;

        for (sector_number, deals) in sector_deal_ids {
            let mut new_deals = deals.clone();
            let existing_deal_ids = sector_deals
                .get(sector_number)
                .context_code(ExitCode::USR_ILLEGAL_STATE, "failed to read sector deals")?;
            if let Some(existing_deal_ids) = existing_deal_ids {
                new_deals.extend(existing_deal_ids.iter());
            }
            new_deals.sort();
            new_deals.dedup();
            sector_deals
                .set(sector_number, new_deals)
                .with_context_code(ExitCode::USR_ILLEGAL_STATE, || {
                    format!("failed to set sector deals for {} {}", provider, sector_number)
                })?;
        }

        save_provider_sector_deals(&mut provider_sectors, provider, &mut sector_deals)?;
        self.save_provider_sectors(&mut provider_sectors)?;
        Ok(())
    }

    // Reads and removes the sector deals mapping for an array of sector numbers,
    pub fn pop_sector_deal_ids(
        &mut self,
        store: &impl Blockstore,
        provider: ActorID,
        sector_numbers: impl Iterator<Item = SectorNumber>,
    ) -> Result<Vec<DealID>, ActorError> {
        let mut provider_sectors = self.load_provider_sectors(store)?;
        let mut sector_deals = load_provider_sector_deals(store, &provider_sectors, provider)?;

        let mut popped_sector_deals = Vec::new();
        let mut flush = false;
        for sector_number in sector_numbers {
            let deals: Option<Vec<DealID>> = sector_deals
                .delete(&sector_number)
                .with_context(|| format!("provider {}", provider))?;
            if let Some(deals) = deals {
                popped_sector_deals.extend(deals.iter());
                flush = true;
            }
        }

        // Flush if any of the requested sectors were found.
        if flush {
            if sector_deals.is_empty() {
                // Remove from top-level map
                provider_sectors
                    .delete(&provider)
                    .with_context_code(ExitCode::USR_ILLEGAL_STATE, || {
                        format!("failed to delete sector deals for {}", provider)
                    })?;
            } else {
                save_provider_sector_deals(&mut provider_sectors, provider, &mut sector_deals)?;
            }
            self.save_provider_sectors(&mut provider_sectors)?;
        }

        Ok(popped_sector_deals)
    }

    // Removes specified deals from the sector deals mapping.
    // Missing deals are ignored.
    pub fn remove_sector_deal_ids(
        &mut self,
        store: &impl Blockstore,
        provider_sector_deal_ids: &BTreeMap<ActorID, BTreeMap<SectorNumber, Vec<DealID>>>,
    ) -> Result<(), ActorError> {
        let mut provider_sectors = self.load_provider_sectors(store)?;
        for (provider, sector_deal_ids) in provider_sector_deal_ids {
            let mut sector_deals = load_provider_sector_deals(store, &provider_sectors, *provider)?;
            for (sector_number, deals_to_remove) in sector_deal_ids {
                let existing_deal_ids = sector_deals
                    .get(sector_number)
                    .context_code(ExitCode::USR_ILLEGAL_STATE, "failed to read sector deals")?;
                if let Some(existing_deal_ids) = existing_deal_ids {
                    // The filter below is a linear scan of deals_to_remove.
                    // This is expected to be a small list, often a singleton, so is usually
                    // pretty fast.
                    // Loading into a HashSet could be an improvement for large collections of deals
                    // in a single sector being removed at one time.
                    let new_deals = existing_deal_ids
                        .iter()
                        .filter(|deal_id| !deals_to_remove.contains(*deal_id))
                        .cloned()
                        .collect();

                    sector_deals
                        .set(sector_number, new_deals)
                        .with_context_code(ExitCode::USR_ILLEGAL_STATE, || {
                            format!("failed to set sector deals for {} {}", provider, sector_number)
                        })?;
                }
            }
            save_provider_sector_deals(&mut provider_sectors, *provider, &mut sector_deals)?;
        }
        self.save_provider_sectors(&mut provider_sectors)?;
        Ok(())
    }

    pub fn load_provider_sectors<BS>(&self, store: BS) -> Result<ProviderSectorsMap<BS>, ActorError>
    where
        BS: Blockstore,
    {
        ProviderSectorsMap::load(
            store,
            &self.provider_sectors,
            PROVIDER_SECTORS_CONFIG,
            "provider sectors",
        )
    }

    fn save_provider_sectors<BS>(
        &mut self,
        provider_sectors: &mut ProviderSectorsMap<BS>,
    ) -> Result<(), ActorError>
    where
        BS: Blockstore,
    {
        self.provider_sectors = provider_sectors.flush()?;
        Ok(())
    }

    /// Delete proposal and state simultaneously.
    pub fn remove_completed_deal<BS>(
        &mut self,
        store: &BS,
        deal_id: DealID,
    ) -> Result<(), ActorError>
    where
        BS: Blockstore,
    {
        let state = self.remove_deal_state(store, deal_id)?;
        if state.is_none() {
            return Err(actor_error!(illegal_state, "failed to delete deal state: does not exist"));
        }
        let proposal = self.remove_proposal(store, deal_id)?;
        if proposal.is_none() {
            return Err(actor_error!(
                illegal_state,
                "failed to delete deal proposal: does not exist"
            ));
        }
        Ok(())
    }

    /// Given a DealProposal, checks that the corresponding deal has activated
    /// If not, checks that the deal is past its activation epoch and performs cleanup
    pub fn get_active_deal_or_process_timeout<BS>(
        &mut self,
        store: &BS,
        curr_epoch: ChainEpoch,
        deal_id: DealID,
        deal_proposal: &DealProposal,
        dcid: &Cid,
    ) -> Result<LoadDealState, ActorError>
    where
        BS: Blockstore,
    {
        let deal_state = self.find_deal_state(store, deal_id)?;

        match deal_state {
            Some(deal_state) => Ok(LoadDealState::Loaded(deal_state)),
            None => {
                // deal_id called too early
                if curr_epoch < deal_proposal.start_epoch {
                    return Ok(LoadDealState::TooEarly);
                }

                // if not activated, the proposal has timed out
                let slashed = self.process_deal_init_timed_out(store, deal_proposal)?;

                // delete the proposal (but not state, which doesn't exist)
                let deleted = self.remove_proposal(store, deal_id)?;
                if deleted.is_none() {
                    return Err(actor_error!(
                        illegal_state,
                        format!(
                            "failed to delete deal {} proposal {}: does not exist",
                            deal_id, dcid
                        )
                    ));
                }

                // delete pending deal cid
                self.remove_pending_deal(store, *dcid)?.ok_or_else(|| {
                    actor_error!(
                        illegal_state,
                        format!(
                            "failed to delete pending deal {}: cid {} does not exist",
                            deal_id, dcid
                        )
                    )
                })?;

                // delete pending deal allocation id (if present)
                self.remove_pending_deal_allocation_id(store, deal_id)?;

                Ok(LoadDealState::ProposalExpired(slashed))
            }
        }
    }

    ////////////////////////////////////////////////////////////////////////////////
    // Deal state operations
    ////////////////////////////////////////////////////////////////////////////////

    // TODO: change return value when marked-for-termination sectors are cleared from state
    // https://github.com/filecoin-project/builtin-actors/issues/1388
    // drop slash_amount, bool return value indicates a completed deal
    pub fn process_deal_update<BS>(
        &mut self,
        store: &BS,
        state: &DealState,
        deal: &DealProposal,
        deal_cid: &Cid,
        epoch: ChainEpoch,
    ) -> Result<
        (
            /* slash_amount */ TokenAmount,
            /* payment_amount */ TokenAmount,
            /* is_deal_completed */ bool,
            /* remove */ bool,
        ),
        ActorError,
    >
    where
        BS: Blockstore,
    {
        let ever_updated = state.last_updated_epoch != EPOCH_UNDEFINED;

        // seeing a slashed deal here will eventually be an unreachable state
        // during the transition to synchronous deal termination there may be marked-for-termination
        // deals that have not been processed in cron yet
        // https://github.com/filecoin-project/builtin-actors/issues/1388
        // TODO: remove this and calculations below that assume deals can be slashed
        let ever_slashed = state.slash_epoch != EPOCH_UNDEFINED;

        if !ever_updated {
            // pending deal might have been removed by manual settlement or cron so we don't care if it's missing
            self.remove_pending_deal(store, *deal_cid)?;
        }

        // if the deal was ever updated, make sure it didn't happen in the future
        if ever_updated && state.last_updated_epoch > epoch {
            return Err(actor_error!(
                illegal_state,
                "deal updated at future epoch {}",
                state.last_updated_epoch
            ));
        }

        // this is a safe no-op but can happen if a storage provider calls settle_deal_payments too early
        if deal.start_epoch > epoch {
            return Ok((TokenAmount::zero(), TokenAmount::zero(), false, false));
        }

        let payment_end_epoch = if ever_slashed {
            if epoch < state.slash_epoch {
                return Err(actor_error!(
                    illegal_state,
                    "current epoch less than deal slash epoch {}",
                    state.slash_epoch
                ));
            }
            if state.slash_epoch > deal.end_epoch {
                return Err(actor_error!(
                    illegal_state,
                    "deal slash epoch {} after deal end {}",
                    state.slash_epoch,
                    deal.end_epoch
                ));
            }
            state.slash_epoch
        } else {
            std::cmp::min(deal.end_epoch, epoch)
        };

        let payment_start_epoch = if ever_updated && state.last_updated_epoch > deal.start_epoch {
            state.last_updated_epoch
        } else {
            deal.start_epoch
        };

        let num_epochs_elapsed = payment_end_epoch - payment_start_epoch;

        let elapsed_payment = &deal.storage_price_per_epoch * num_epochs_elapsed;
        if elapsed_payment.is_positive() {
            self.transfer_balance(store, &deal.client, &deal.provider, &elapsed_payment)?;
        }

        // TODO: remove handling of terminated deals *after* transition to synchronous deal termination
        // at that point, this function can be modified to return a bool only, indicating whether the deal is completed
        // https://github.com/filecoin-project/builtin-actors/issues/1388
        if ever_slashed {
            // unlock client collateral and locked storage fee
            let payment_remaining = deal_get_payment_remaining(deal, state.slash_epoch)?;

            // Unlock remaining storage fee
            self.unlock_balance(store, &deal.client, &payment_remaining, Reason::ClientStorageFee)
                .context("unlocking client storage fee")?;

            // Unlock client collateral
            self.unlock_balance(
                store,
                &deal.client,
                &deal.client_collateral,
                Reason::ClientCollateral,
            )
            .context("unlocking client collateral")?;

            // slash provider collateral
            let slashed = deal.provider_collateral.clone();
            self.slash_balance(store, &deal.provider, &slashed, Reason::ProviderCollateral)
                .context("slashing balance")?;

            return Ok((slashed, payment_remaining + elapsed_payment, false, true));
        }

        if epoch >= deal.end_epoch {
            self.process_deal_expired(store, deal, state)?;
            return Ok((TokenAmount::zero(), elapsed_payment, true, true));
        }

        Ok((TokenAmount::zero(), elapsed_payment, false, false))
    }

    pub fn process_slashed_deal<BS>(
        &mut self,
        store: &BS,
        proposal: &DealProposal,
        state: &DealState,
    ) -> Result<TokenAmount, ActorError>
    where
        BS: Blockstore,
    {
        // make payments for epochs until termination
        let payment_start_epoch = max(proposal.start_epoch, state.last_updated_epoch);
        let payment_end_epoch = min(proposal.end_epoch, state.slash_epoch);
        let num_epochs_elapsed = max(0, payment_end_epoch - payment_start_epoch);
        let total_payment = &proposal.storage_price_per_epoch * num_epochs_elapsed;
        if total_payment.is_positive() {
            self.transfer_balance(store, &proposal.client, &proposal.provider, &total_payment)?;
        }

        // unlock client collateral and locked storage fee
        let payment_remaining = deal_get_payment_remaining(proposal, state.slash_epoch)?;

        // Unlock remaining storage fee
        self.unlock_balance(store, &proposal.client, &payment_remaining, Reason::ClientStorageFee)
            .context("unlocking client storage fee")?;

        // Unlock client collateral
        self.unlock_balance(
            store,
            &proposal.client,
            &proposal.client_collateral,
            Reason::ClientCollateral,
        )
        .context("unlocking client collateral")?;

        // slash provider collateral
        let slashed = proposal.provider_collateral.clone();
        self.slash_balance(store, &proposal.provider, &slashed, Reason::ProviderCollateral)
            .context("slashing balance")?;

        Ok(slashed)
    }

    /// Deal start deadline elapsed without appearing in a proven sector.
    /// Slash a portion of provider's collateral, and unlock remaining collaterals
    /// for both provider and client.
    pub fn process_deal_init_timed_out<BS>(
        &mut self,
        store: &BS,
        deal: &DealProposal,
    ) -> Result<TokenAmount, ActorError>
    where
        BS: Blockstore,
    {
        self.unlock_balance(
            store,
            &deal.client,
            &deal.total_storage_fee(),
            Reason::ClientStorageFee,
        )
        .context("unlocking client storage fee")?;

        self.unlock_balance(store, &deal.client, &deal.client_collateral, Reason::ClientCollateral)
            .context("unlocking client collateral")?;

        let amount_slashed =
            collateral_penalty_for_deal_activation_missed(deal.provider_collateral.clone());
        let amount_remaining = deal.provider_balance_requirement() - &amount_slashed;

        self.slash_balance(store, &deal.provider, &amount_slashed, Reason::ProviderCollateral)
            .context("slashing balance")?;

        self.unlock_balance(store, &deal.provider, &amount_remaining, Reason::ProviderCollateral)
            .context("unlocking deal provider balance")?;

        Ok(amount_slashed)
    }

    /// Normal expiration. Unlock collaterals for both miner and client.
    fn process_deal_expired<BS>(
        &mut self,
        store: &BS,
        deal: &DealProposal,
        state: &DealState,
    ) -> Result<(), ActorError>
    where
        BS: Blockstore,
    {
        if state.sector_start_epoch == EPOCH_UNDEFINED {
            return Err(actor_error!(illegal_state, "start sector epoch undefined"));
        }

        self.unlock_balance(
            store,
            &deal.provider,
            &deal.provider_collateral,
            Reason::ProviderCollateral,
        )
        .context("unlocking deal provider balance")?;

        self.unlock_balance(store, &deal.client, &deal.client_collateral, Reason::ClientCollateral)
            .context("unlocking deal client balance")?;

        Ok(())
    }

    pub fn generate_storage_deal_id(&mut self) -> DealID {
        let ret = self.next_id;
        self.next_id += 1;
        ret
    }

    // Return true when the funds in escrow for the input address can cover an additional lockup of amountToLock
    pub fn balance_covered<BS>(
        &self,
        store: &BS,
        addr: Address,
        amount_to_lock: &TokenAmount,
    ) -> Result<bool, ActorError>
    where
        BS: Blockstore,
    {
        let escrow_table = BalanceTable::from_root(store, &self.escrow_table, "escrow table")?;
        let locked_table = BalanceTable::from_root(store, &self.locked_table, "locked table")?;

        let escrow_balance = escrow_table.get(&addr)?;
        let prev_locked = locked_table.get(&addr)?;
        Ok((prev_locked + amount_to_lock) <= escrow_balance)
    }

    fn maybe_lock_balance<BS>(
        &mut self,
        store: &BS,
        addr: &Address,
        amount: &TokenAmount,
    ) -> Result<(), ActorError>
    where
        BS: Blockstore,
    {
        if amount.is_negative() {
            return Err(actor_error!(illegal_state, "cannot lock negative amount {}", amount));
        }

        let escrow_table = BalanceTable::from_root(store, &self.escrow_table, "escrow table")?;
        let mut locked_table = BalanceTable::from_root(store, &self.locked_table, "locked table")?;

        let prev_locked = locked_table.get(addr)?;
        let escrow_balance = escrow_table.get(addr)?;
        if &prev_locked + amount > escrow_balance {
            return Err(actor_error!(insufficient_funds;
                    "not enough balance to lock for addr{}: \
                    escrow balance {} < prev locked {} + amount {}",
                    addr, escrow_balance, prev_locked, amount));
        }

        locked_table.add(addr, amount)?;
        self.locked_table = locked_table.root()?;
        Ok(())
    }

    pub fn lock_client_and_provider_balances<BS>(
        &mut self,
        store: &BS,
        proposal: &DealProposal,
    ) -> Result<(), ActorError>
    where
        BS: Blockstore,
    {
        self.maybe_lock_balance(store, &proposal.client, &proposal.client_balance_requirement())
            .context("locking client funds")?;
        self.maybe_lock_balance(store, &proposal.provider, &proposal.provider_collateral)
            .context("locking provider funds")?;

        self.total_client_locked_collateral += &proposal.client_collateral;
        self.total_client_storage_fee += proposal.total_storage_fee();
        self.total_provider_locked_collateral += &proposal.provider_collateral;
        Ok(())
    }

    fn unlock_balance<BS>(
        &mut self,
        store: &BS,
        addr: &Address,
        amount: &TokenAmount,
        lock_reason: Reason,
    ) -> Result<(), ActorError>
    where
        BS: Blockstore,
    {
        if amount.is_negative() {
            return Err(actor_error!(illegal_state, "unlock negative amount: {}", amount));
        }

        let mut locked_table = BalanceTable::from_root(store, &self.locked_table, "locked table")?;
        locked_table.must_subtract(addr, amount).context("unlocking balance")?;

        match lock_reason {
            Reason::ClientCollateral => {
                self.total_client_locked_collateral -= amount;
            }
            Reason::ClientStorageFee => {
                self.total_client_storage_fee -= amount;
            }
            Reason::ProviderCollateral => {
                self.total_provider_locked_collateral -= amount;
            }
        };

        self.locked_table = locked_table.root()?;
        Ok(())
    }

    /// move funds from locked in client to available in provider
    fn transfer_balance<BS>(
        &mut self,
        store: &BS,
        from_addr: &Address,
        to_addr: &Address,
        amount: &TokenAmount,
    ) -> Result<(), ActorError>
    where
        BS: Blockstore,
    {
        if amount.is_negative() {
            return Err(actor_error!(illegal_state, "transfer negative amount: {}", amount));
        }

        let mut escrow_table = BalanceTable::from_root(store, &self.escrow_table, "escrow table")?;

        // Subtract from locked and escrow tables
        escrow_table.must_subtract(from_addr, amount)?;
        self.unlock_balance(store, from_addr, amount, Reason::ClientStorageFee)
            .context("unlocking client balance")?;

        // Add subtracted amount to the recipient
        escrow_table.add(to_addr, amount)?;
        self.escrow_table = escrow_table.root()?;
        Ok(())
    }

    fn slash_balance<BS>(
        &mut self,
        store: &BS,
        addr: &Address,
        amount: &TokenAmount,
        lock_reason: Reason,
    ) -> Result<(), ActorError>
    where
        BS: Blockstore,
    {
        if amount.is_negative() {
            return Err(actor_error!(illegal_state, "negative amount to slash: {}", amount));
        }

        let mut escrow_table = BalanceTable::from_root(store, &self.escrow_table, "escrow table")?;

        // Subtract from locked and escrow tables
        escrow_table.must_subtract(addr, amount)?;
        self.escrow_table = escrow_table.root()?;
        self.unlock_balance(store, addr, amount, lock_reason)
    }
}

pub enum LoadDealState {
    TooEarly,
    ProposalExpired(/* slashed_amount */ TokenAmount),
    Loaded(DealState),
}

pub fn deal_get_payment_remaining(
    deal: &DealProposal,
    mut slash_epoch: ChainEpoch,
) -> Result<TokenAmount, ActorError> {
    if slash_epoch > deal.end_epoch {
        return Err(actor_error!(
            illegal_state,
            "deal slash epoch {} after end epoch {}",
            slash_epoch,
            deal.end_epoch
        ));
    }

    // Payments are always for start -> end epoch irrespective of when the deal is slashed.
    slash_epoch = std::cmp::max(slash_epoch, deal.start_epoch);

    let duration_remaining = deal.end_epoch - slash_epoch;
    if duration_remaining < 0 {
        return Err(actor_error!(
            illegal_state,
            "deal remaining duration negative: {}",
            duration_remaining
        ));
    }

    Ok(&deal.storage_price_per_epoch * duration_remaining as u64)
}

pub fn get_proposal<BS: Blockstore>(
    proposals: &DealArray<BS>,
    id: DealID,
    next_id: DealID,
) -> Result<DealProposal, ActorError> {
    let found = find_proposal(proposals, id)?.ok_or_else(|| {
        if id < next_id {
            // If the deal ID has been used, it must have been cleaned up.
            ActorError::unchecked(EX_DEAL_EXPIRED, format!("deal {} expired", id))
        } else {
            // Never been published.
            ActorError::not_found(format!("no such deal {}", id))
        }
    })?;
    Ok(found)
}

pub fn find_proposal<BS>(
    proposals: &DealArray<BS>,
    deal_id: DealID,
) -> Result<Option<DealProposal>, ActorError>
where
    BS: Blockstore,
{
    let proposal = proposals.get(deal_id).with_context_code(ExitCode::USR_ILLEGAL_STATE, || {
        format!("failed to load deal proposal {}", deal_id)
    })?;
    Ok(proposal.cloned())
}

pub fn find_deal_state<BS>(
    states: &DealMetaArray<BS>,
    deal_id: DealID,
) -> Result<Option<DealState>, ActorError>
where
    BS: Blockstore,
{
    let state = states.get(deal_id).with_context_code(ExitCode::USR_ILLEGAL_STATE, || {
        format!("failed to load deal state {}", deal_id)
    })?;
    Ok(state.cloned())
}

pub fn load_provider_sector_deals<BS>(
    store: BS,
    provider_sectors: &ProviderSectorsMap<BS>,
    provider: ActorID,
) -> Result<SectorDealsMap<BS>, ActorError>
where
    BS: Blockstore,
{
    let sectors_root = (*provider_sectors).get(&provider)?;
    let sector_deals = if let Some(sectors_root) = sectors_root {
        SectorDealsMap::load(store, sectors_root, SECTOR_DEALS_CONFIG, "sector deals")
            .with_context(|| format!("provider {}", provider))?
    } else {
        SectorDealsMap::empty(store, SECTOR_DEALS_CONFIG, "empty")
    };
    Ok(sector_deals)
}

fn save_provider_sector_deals<BS>(
    provider_sectors: &mut ProviderSectorsMap<BS>,
    provider: ActorID,
    sector_deals: &mut SectorDealsMap<BS>,
) -> Result<(), ActorError>
where
    BS: Blockstore,
{
    let sectors_root = sector_deals.flush()?;
    provider_sectors.set(&provider, sectors_root)?;
    Ok(())
}
