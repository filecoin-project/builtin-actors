// Copyright 2019-2022 ChainSafe Systems
// SPDX-License-Identifier: Apache-2.0, MIT

use crate::balance_table::BalanceTable;
use crate::ext::verifreg::AllocationID;
use anyhow::anyhow;
use cid::Cid;
use fil_actors_runtime::runtime::Policy;
use fil_actors_runtime::{
    actor_error, make_empty_map, make_map_with_root_and_bitwidth, ActorDowncast, ActorError, Array,
    Set, SetMultimap,
};
use fvm_ipld_blockstore::Blockstore;
use fvm_ipld_encoding::tuple::*;
use fvm_ipld_encoding::Cbor;
use fvm_ipld_hamt::BytesKey;
use fvm_shared::address::Address;
use fvm_shared::clock::{ChainEpoch, EPOCH_UNDEFINED};
use fvm_shared::deal::DealID;
use fvm_shared::econ::TokenAmount;
use fvm_shared::error::ExitCode;
use fvm_shared::HAMT_BIT_WIDTH;
use num_traits::Zero;
use std::collections::BTreeMap;

use super::policy::*;
use super::types::*;
use super::{DealProposal, DealState};

fn deal_get_payment_remaining(
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

pub(super) enum Reason {
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
    pub pending_deal_allocation_ids: Cid, // HAMT[DealID]AllocationID
}

impl Cbor for State {}

impl State {
    pub fn new<BS: Blockstore>(store: &BS) -> anyhow::Result<Self> {
        let empty_proposals_array =
            Array::<(), BS>::new_with_bit_width(store, PROPOSALS_AMT_BITWIDTH)
                .flush()
                .map_err(|e| anyhow!("Failed to create empty proposals array: {}", e))?;
        let empty_states_array = Array::<(), BS>::new_with_bit_width(store, STATES_AMT_BITWIDTH)
            .flush()
            .map_err(|e| anyhow!("Failed to create empty states array: {}", e))?;

        let empty_pending_proposals_map = make_empty_map::<_, ()>(store, HAMT_BIT_WIDTH)
            .flush()
            .map_err(|e| anyhow!("Failed to create empty pending proposals map state: {}", e))?;
        let empty_balance_table = BalanceTable::new(store)
            .root()
            .map_err(|e| anyhow!("Failed to create empty balance table map: {}", e))?;
        let empty_deal_ops_hamt = SetMultimap::new(store)
            .root()
            .map_err(|e| anyhow!("Failed to create empty multiset: {}", e))?;
        let empty_pending_deal_allocation_map =
            make_empty_map::<_, AllocationID>(store, HAMT_BIT_WIDTH).flush().map_err(|e| {
                anyhow!("Failed to create empty pending deal allocation map: {}", e)
            })?;
        Ok(Self {
            proposals: empty_proposals_array,
            states: empty_states_array,
            pending_proposals: empty_pending_proposals_map,
            escrow_table: empty_balance_table,
            locked_table: empty_balance_table,
            next_id: 0,
            deal_ops_by_epoch: empty_deal_ops_hamt,
            last_cron: EPOCH_UNDEFINED,

            total_client_locked_collateral: TokenAmount::default(),
            total_provider_locked_collateral: TokenAmount::default(),
            total_client_storage_fee: TokenAmount::default(),
            pending_deal_allocation_ids: empty_pending_deal_allocation_map,
        })
    }

    pub fn total_locked(&self) -> TokenAmount {
        &self.total_client_locked_collateral
            + &self.total_provider_locked_collateral
            + &self.total_client_storage_fee
    }

    pub(super) fn deal_states_get<BS>(
        &self,
        store: &BS,
        deal_id: DealID,
    ) -> anyhow::Result<Option<DealState>>
    where
        BS: Blockstore,
    {
        let deal_states = DealMetaArray::load(&self.states, store)
            .map_err(|e| e.downcast_wrap("failed to load deal state array"))?;

        let deal_state = deal_states
            .get(deal_id)
            .map_err(|e| e.downcast_wrap(&format!("no such deal state {}", deal_id)))?;

        Ok(deal_state.cloned())
    }

    pub(super) fn deal_states_update<BS>(
        &mut self,
        store: &BS,
        new_deal_states: &[(DealID, DealState)],
    ) -> anyhow::Result<()>
    where
        BS: Blockstore,
    {
        let mut deal_states = DealMetaArray::load(&self.states, store)
            .map_err(|e| e.downcast_wrap("failed to load deal proposal array"))?;

        new_deal_states.iter().try_for_each(|(id, deal_state)| -> anyhow::Result<()> {
            deal_states
                .set(*id, *deal_state)
                .map_err(|e| e.downcast_wrap("failed to set deal state"))?;
            Ok(())
        })?;

        self.states =
            deal_states.flush().map_err(|e| e.downcast_wrap("failed to flush deal states"))?;

        Ok(())
    }

    pub(super) fn deal_states_delete<BS>(
        &mut self,
        store: &BS,
        deal_id: DealID,
    ) -> anyhow::Result<Option<DealState>>
    where
        BS: Blockstore,
    {
        let mut deal_states = DealMetaArray::load(&self.states, store)
            .map_err(|e| e.downcast_wrap("failed to load deal proposal array"))?;

        let rval_deal_state = deal_states
            .delete(deal_id)
            .map_err(|e| e.downcast_wrap("failed to delete deal state"))?;

        self.states =
            deal_states.flush().map_err(|e| e.downcast_wrap("failed to flush deal states"))?;

        Ok(rval_deal_state)
    }

    pub(super) fn deal_proposals_get<BS>(
        &self,
        store: &BS,
        deal_id: DealID,
    ) -> anyhow::Result<Option<DealProposal>>
    where
        BS: Blockstore,
    {
        let deal_proposals = DealArray::load(&self.proposals, store)
            .map_err(|e| e.downcast_wrap("failed to load deal proposal array"))?;

        let proposal = deal_proposals
            .get(deal_id)
            .map_err(|e| e.downcast_wrap(&format!("no such deal proposal {}", deal_id)))?;

        Ok(proposal)
    }

    pub(super) fn deal_proposals_delete<BS>(
        &mut self,
        store: &BS,
        deal_id: DealID,
    ) -> anyhow::Result<Option<DealProposal>>
    where
        BS: Blockstore,
    {
        let mut deal_proposals = DealArray::load(&self.proposals, store)
            .map_err(|e| e.downcast_wrap("failed to load deal proposal array"))?;

        let proposal = deal_proposals
            .delete(deal_id)
            .map_err(|e| e.downcast_wrap(&format!("no such deal proposal {}", deal_id)))?;

        self.proposals = deal_proposals
            .flush()
            .map_err(|e| e.downcast_wrap("failed to flush deal proposals"))?;

        Ok(proposal)
    }

    pub(super) fn deal_proposals_update<BS>(
        &mut self,
        store: &BS,
        new_deal_proposals: &[(DealID, DealProposal)],
    ) -> anyhow::Result<()>
    where
        BS: Blockstore,
    {
        let mut deal_proposals = DealArray::load(&self.proposals, store)
            .map_err(|e| e.downcast_wrap("failed to load deal proposal array"))?;

        new_deal_proposals.iter().try_for_each(|(id, proposal)| -> anyhow::Result<()> {
            deal_proposals
                .set(*id, proposal.clone())
                .map_err(|e| e.downcast_wrap("failed to set deal proposal"))?;
            Ok(())
        })?;

        self.proposals = deal_proposals
            .flush()
            .map_err(|e| e.downcast_wrap("failed to flush deal proposals"))?;

        Ok(())
    }

    pub(super) fn pending_deal_allocation_ids_update<BS>(
        &mut self,
        store: &BS,
        new_pending_deal_allocation_ids: &[(BytesKey, AllocationID)],
    ) -> anyhow::Result<()>
    where
        BS: Blockstore,
    {
        let mut pending_deal_allocation_ids = make_map_with_root_and_bitwidth(
            &self.pending_deal_allocation_ids,
            store,
            HAMT_BIT_WIDTH,
        )
        .map_err(|e| e.downcast_wrap("failed to load pending deal allocation id's"))?;

        new_pending_deal_allocation_ids.iter().try_for_each(
            |(id, allocation_id)| -> anyhow::Result<()> {
                pending_deal_allocation_ids
                    .set(id.clone(), *allocation_id)
                    .map_err(|e| e.downcast_wrap("failed to set pending deal allocation id"))?;
                Ok(())
            },
        )?;

        self.pending_deal_allocation_ids = pending_deal_allocation_ids
            .flush()
            .map_err(|e| e.downcast_wrap("failed to flush pending deal allocation id"))?;

        Ok(())
    }

    pub(super) fn pending_deal_allocation_ids_delete<BS>(
        &mut self,
        store: &BS,
        deal_id_key: &BytesKey,
    ) -> anyhow::Result<Option<(BytesKey, AllocationID)>>
    where
        BS: Blockstore,
    {
        let mut pending_deal_allocation_ids = make_map_with_root_and_bitwidth(
            &self.pending_deal_allocation_ids,
            store,
            HAMT_BIT_WIDTH,
        )
        .map_err(|e| e.downcast_wrap("failed to load pending deal allocation id's"))?;

        let rval_allocation_id = pending_deal_allocation_ids
            .delete(deal_id_key)
            .map_err(|e| e.downcast_wrap(&format!("no such deal proposal {:#?}", deal_id_key)))?;

        self.pending_deal_allocation_ids = pending_deal_allocation_ids
            .flush()
            .map_err(|e| e.downcast_wrap("failed to flush deal proposals"))?;

        Ok(rval_allocation_id)
    }

    pub(super) fn deals_by_epoch_update<BS>(
        &mut self,
        store: &BS,
        new_deals_by_epoch: &[(ChainEpoch, DealID)],
    ) -> anyhow::Result<()>
    where
        BS: Blockstore,
    {
        let mut deals_by_epoch = SetMultimap::from_root(store, &self.deal_ops_by_epoch)
            .map_err(|e| e.downcast_wrap("failed to load deals by epoch"))?;

        new_deals_by_epoch.iter().try_for_each(|(epoch, id)| -> anyhow::Result<()> {
            deals_by_epoch.put(*epoch, *id).map_err(|e| e.downcast_wrap("failed to set deal"))?;
            Ok(())
        })?;

        self.deal_ops_by_epoch =
            deals_by_epoch.root().map_err(|e| e.downcast_wrap("failed to flush deals by epoch"))?;

        Ok(())
    }

    pub(super) fn deals_by_epoch_update_many<BS>(
        &mut self,
        store: &BS,
        new_deals_by_epoch: &BTreeMap<ChainEpoch, Vec<DealID>>,
    ) -> anyhow::Result<()>
    where
        BS: Blockstore,
    {
        let mut deals_by_epoch = SetMultimap::from_root(store, &self.deal_ops_by_epoch)
            .map_err(|e| e.downcast_wrap("failed to load deals by epoch"))?;

        new_deals_by_epoch.iter().try_for_each(|(epoch, deals)| -> anyhow::Result<()> {
            deals_by_epoch.put_many(*epoch, deals).map_err(|e| {
                e.downcast_default(
                    ExitCode::USR_ILLEGAL_STATE,
                    format!("failed to reinsert deal IDs for epoch {}", epoch),
                )
            })?;
            Ok(())
        })?;

        self.deal_ops_by_epoch =
            deals_by_epoch.root().map_err(|e| e.downcast_wrap("failed to flush deals by epoch"))?;

        Ok(())
    }

    pub(super) fn deals_for_epoch_get<BS>(
        &self,
        store: &BS,
        key: ChainEpoch,
    ) -> anyhow::Result<Vec<DealID>>
    where
        BS: Blockstore,
    {
        let mut deal_ids = Vec::new();

        let deals_by_epoch = SetMultimap::from_root(store, &self.deal_ops_by_epoch)
            .map_err(|e| e.downcast_wrap("failed to load deals by epoch"))?;

        deals_by_epoch
            .for_each(key, |deal_id| {
                deal_ids.push(deal_id);
                Ok(())
            })
            .map_err(|e| {
                e.downcast_default(ExitCode::USR_ILLEGAL_STATE, "failed to set deal state")
            })?;

        Ok(deal_ids)
    }

    pub(super) fn deals_by_epoch_delete<BS>(
        &mut self,
        store: &BS,
        new_deals_by_epoch: &[ChainEpoch],
    ) -> anyhow::Result<()>
    where
        BS: Blockstore,
    {
        let mut deals_by_epoch = SetMultimap::from_root(store, &self.deal_ops_by_epoch)
            .map_err(|e| e.downcast_wrap("failed to load deals by epoch"))?;

        new_deals_by_epoch.iter().try_for_each(|epoch| -> anyhow::Result<()> {
            deals_by_epoch.remove_all(*epoch).map_err(|e| {
                e.downcast_default(
                    ExitCode::USR_ILLEGAL_STATE,
                    format!("failed to delete deal ops for epoch {}", epoch),
                )
            })?;
            Ok(())
        })?;

        self.deal_ops_by_epoch =
            deals_by_epoch.root().map_err(|e| e.downcast_wrap("failed to flush deals by epoch"))?;

        Ok(())
    }

    pub(super) fn escrow_table_add_balance<BS>(
        &mut self,
        store: &BS,
        addr: &Address,
        amount: &TokenAmount,
    ) -> anyhow::Result<()>
    where
        BS: Blockstore,
    {
        let mut escrow_table = BalanceTable::from_root(store, &self.escrow_table)
            .map_err(|e| e.downcast_wrap("failed to load escrow table"))?;

        escrow_table.add(addr, amount).map_err(|e| {
            e.downcast_default(ExitCode::USR_ILLEGAL_STATE, "failed to add escrow table")
        })?;

        self.escrow_table =
            escrow_table.root().map_err(|e| e.downcast_wrap("failed to flush escrow table"))?;

        Ok(())
    }

    pub(super) fn escrow_table_withdraw_balance<BS>(
        &mut self,
        store: &BS,
        addr: &Address,
        amount: &TokenAmount,
    ) -> anyhow::Result<TokenAmount>
    where
        BS: Blockstore,
    {
        let mut escrow_table = BalanceTable::from_root(store, &self.escrow_table)
            .map_err(|e| e.downcast_wrap("failed to load escrow table"))?;

        let locked_table = BalanceTable::from_root(store, &self.locked_table)
            .map_err(|e| e.downcast_wrap("failed to load locked table"))?;

        let min_balance = locked_table.get(addr).map_err(|e| {
            e.downcast_default(ExitCode::USR_ILLEGAL_STATE, "failed to get locked balance")
        })?;

        let ex = escrow_table.subtract_with_minimum(addr, amount, &min_balance).map_err(|e| {
            e.downcast_default(ExitCode::USR_ILLEGAL_STATE, "failed to subtract from escrow table")
        })?;

        self.escrow_table =
            escrow_table.root().map_err(|e| e.downcast_wrap("failed to flush escrow table"))?;

        Ok(ex)
    }

    pub(super) fn pending_deals_has<BS>(&self, store: &BS, key: Cid) -> anyhow::Result<bool>
    where
        BS: Blockstore,
    {
        let pending_deals = Set::from_root(store, &self.pending_proposals)
            .map_err(|e| e.downcast_wrap("failed to get pending deals"))?;

        let rval = pending_deals.has(&key.to_bytes()).map_err(|e| {
            e.downcast_default(ExitCode::USR_ILLEGAL_STATE, "failed to get pending deals")
        })?;
        Ok(rval)
    }

    pub(super) fn pending_deals_update<BS>(
        &mut self,
        store: &BS,
        new_pending_deals: &[Cid],
    ) -> anyhow::Result<()>
    where
        BS: Blockstore,
    {
        let mut pending_deals = Set::from_root(store, &self.pending_proposals)
            .map_err(|e| e.downcast_wrap("failed to load pending deals"))?;

        new_pending_deals.iter().try_for_each(|key: &Cid| -> anyhow::Result<()> {
            pending_deals
                .put(key.to_bytes().into())
                .map_err(|e| e.downcast_wrap("failed to set deal"))?;
            Ok(())
        })?;

        self.pending_proposals =
            pending_deals.root().map_err(|e| e.downcast_wrap("failed to flush pending deals"))?;

        Ok(())
    }

    pub(super) fn pending_deals_delete<BS>(
        &mut self,
        store: &BS,
        pending_deal_key: Cid,
    ) -> anyhow::Result<Option<()>>
    where
        BS: Blockstore,
    {
        let mut pending_deals = Set::from_root(store, &self.pending_proposals)
            .map_err(|e| e.downcast_wrap("failed to load pending deals"))?;

        let rval_pending_deal =
            pending_deals.delete(&pending_deal_key.to_bytes()).map_err(|e| {
                e.downcast_wrap(&format!("failed to delete pending proposal {}", pending_deal_key))
            })?;

        self.pending_proposals =
            pending_deals.root().map_err(|e| e.downcast_wrap("failed to flush pending deals"))?;

        Ok(rval_pending_deal)
    }

    ////////////////////////////////////////////////////////////////////////////////
    // Deal state operations
    ////////////////////////////////////////////////////////////////////////////////
    #[allow(clippy::too_many_arguments)]
    pub(super) fn update_pending_deal_state<BS>(
        &mut self,
        store: &BS,
        policy: &Policy,
        state: &DealState,
        deal: &DealProposal,
        epoch: ChainEpoch,
    ) -> Result<(TokenAmount, ChainEpoch, bool), ActorError>
    where
        BS: Blockstore,
    {
        let ever_updated = state.last_updated_epoch != EPOCH_UNDEFINED;
        let ever_slashed = state.slash_epoch != EPOCH_UNDEFINED;

        // if the deal was ever updated, make sure it didn't happen in the future
        if ever_updated && state.last_updated_epoch > epoch {
            return Err(actor_error!(
                illegal_state,
                "deal updated at future epoch {}",
                state.last_updated_epoch
            ));
        }

        // This would be the case that the first callback somehow triggers before it is scheduled to
        // This is expected not to be able to happen
        if deal.start_epoch > epoch {
            return Ok((TokenAmount::zero(), EPOCH_UNDEFINED, false));
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

        let total_payment = &deal.storage_price_per_epoch * num_epochs_elapsed;
        if total_payment.is_positive() {
            self.transfer_balance(store, &deal.client, &deal.provider, &total_payment)?;
        }

        if ever_slashed {
            // unlock client collateral and locked storage fee
            let payment_remaining = deal_get_payment_remaining(deal, state.slash_epoch)?;

            // Unlock remaining storage fee
            self.unlock_balance(store, &deal.client, &payment_remaining, Reason::ClientStorageFee)
                .map_err(|e| {
                    e.downcast_default(
                        ExitCode::USR_ILLEGAL_STATE,
                        "failed to unlock remaining client storage fee",
                    )
                })?;

            // Unlock client collateral
            self.unlock_balance(
                store,
                &deal.client,
                &deal.client_collateral,
                Reason::ClientCollateral,
            )
            .map_err(|e| {
                e.downcast_default(
                    ExitCode::USR_ILLEGAL_STATE,
                    "failed to unlock client collateral",
                )
            })?;

            // slash provider collateral
            let slashed = deal.provider_collateral.clone();
            self.slash_balance(store, &deal.provider, &slashed, Reason::ProviderCollateral)
                .map_err(|e| e.downcast_default(ExitCode::USR_ILLEGAL_STATE, "slashing balance"))?;

            return Ok((slashed, EPOCH_UNDEFINED, true));
        }

        if epoch >= deal.end_epoch {
            self.process_deal_expired(store, deal, state)?;
            return Ok((TokenAmount::zero(), EPOCH_UNDEFINED, true));
        }

        // We're explicitly not inspecting the end epoch and may process a deal's expiration late,
        // in order to prevent an outsider from loading a cron tick by activating too many deals
        // with the same end epoch.
        let next = epoch + policy.deal_updates_interval;

        Ok((TokenAmount::zero(), next, false))
    }

    /// Deal start deadline elapsed without appearing in a proven sector.
    /// Slash a portion of provider's collateral, and unlock remaining collaterals
    /// for both provider and client.
    pub(super) fn process_deal_init_timed_out<BS>(
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
        .map_err(|e| {
            e.downcast_default(ExitCode::USR_ILLEGAL_STATE, "failure unlocking client storage fee")
        })?;

        self.unlock_balance(store, &deal.client, &deal.client_collateral, Reason::ClientCollateral)
            .map_err(|e| {
                e.downcast_default(
                    ExitCode::USR_ILLEGAL_STATE,
                    "failure unlocking client collateral",
                )
            })?;

        let amount_slashed =
            collateral_penalty_for_deal_activation_missed(deal.provider_collateral.clone());
        let amount_remaining = deal.provider_balance_requirement() - &amount_slashed;

        self.slash_balance(store, &deal.provider, &amount_slashed, Reason::ProviderCollateral)
            .map_err(|e| {
                e.downcast_default(ExitCode::USR_ILLEGAL_STATE, "failed to slash balance")
            })?;

        self.unlock_balance(store, &deal.provider, &amount_remaining, Reason::ProviderCollateral)
            .map_err(|e| {
            e.downcast_default(
                ExitCode::USR_ILLEGAL_STATE,
                "failed to unlock deal provider balance",
            )
        })?;

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
        .map_err(|e| {
            e.downcast_default(
                ExitCode::USR_ILLEGAL_STATE,
                "failed unlocking deal provider balance",
            )
        })?;

        self.unlock_balance(store, &deal.client, &deal.client_collateral, Reason::ClientCollateral)
            .map_err(|e| {
                e.downcast_default(
                    ExitCode::USR_ILLEGAL_STATE,
                    "failed unlocking deal client balance",
                )
            })?;

        Ok(())
    }

    pub(super) fn generate_storage_deal_id(&mut self) -> DealID {
        let ret = self.next_id;
        self.next_id += 1;
        ret
    }

    // Return true when the funds in escrow for the input address can cover an additional lockup of amountToLock
    pub(super) fn balance_covered<BS>(
        &self,
        store: &BS,
        addr: Address,
        amount_to_lock: &TokenAmount,
    ) -> anyhow::Result<bool>
    where
        BS: Blockstore,
    {
        let escrow_table = BalanceTable::from_root(store, &self.escrow_table)
            .map_err(|e| e.downcast_wrap("failed to load escrow table"))?;

        let locked_table = BalanceTable::from_root(store, &self.locked_table)
            .map_err(|e| e.downcast_wrap("failed to load locked table"))?;

        let escrow_balance = escrow_table.get(&addr).map_err(|e| {
            e.downcast_default(ExitCode::USR_ILLEGAL_STATE, "failed to get escrow balance")
        })?;

        let prev_locked = locked_table.get(&addr).map_err(|e| {
            e.downcast_default(ExitCode::USR_ILLEGAL_STATE, "failed to get locked balance")
        })?;

        Ok((prev_locked + amount_to_lock) <= escrow_balance)
    }

    fn maybe_lock_balance<BS>(
        &mut self,
        store: &BS,
        addr: &Address,
        amount: &TokenAmount,
    ) -> anyhow::Result<()>
    where
        BS: Blockstore,
    {
        if amount.is_negative() {
            return Err(
                actor_error!(illegal_state, "cannot lock negative amount {}", amount).into()
            );
        }

        let escrow_table = BalanceTable::from_root(store, &self.escrow_table)
            .map_err(|e| e.downcast_wrap("failed to load escrow table"))?;

        let mut locked_table = BalanceTable::from_root(store, &self.locked_table)
            .map_err(|e| e.downcast_wrap("failed to load locked table"))?;

        let prev_locked = locked_table.get(addr).map_err(|e| {
            e.downcast_default(ExitCode::USR_ILLEGAL_STATE, "failed to get locked balance")
        })?;

        let escrow_balance = escrow_table.get(addr).map_err(|e| {
            e.downcast_default(ExitCode::USR_ILLEGAL_STATE, "failed to get escrow balance")
        })?;

        if &prev_locked + amount > escrow_balance {
            return Err(actor_error!(insufficient_funds;
                    "not enough balance to lock for addr{}: \
                    escrow balance {} < prev locked {} + amount {}",
                    addr, escrow_balance, prev_locked, amount)
            .into());
        }

        locked_table.add(addr, amount).map_err(|e| {
            e.downcast_default(ExitCode::USR_ILLEGAL_STATE, "failed to add locked balance")
        })?;

        self.locked_table =
            locked_table.root().map_err(|e| e.downcast_wrap("failed to flush locked table"))?;

        Ok(())
    }

    pub(super) fn lock_client_and_provider_balances<BS>(
        &mut self,
        store: &BS,
        proposal: &DealProposal,
    ) -> anyhow::Result<()>
    where
        BS: Blockstore,
    {
        self.maybe_lock_balance(store, &proposal.client, &proposal.client_balance_requirement())
            .map_err(|e| e.downcast_wrap("failed to lock client funds"))?;

        self.maybe_lock_balance(store, &proposal.provider, &proposal.provider_collateral)
            .map_err(|e| e.downcast_wrap("failed to lock provider funds"))?;

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
    ) -> anyhow::Result<()>
    where
        BS: Blockstore,
    {
        if amount.is_negative() {
            return Err(actor_error!(illegal_state, "unlock negative amount: {}", amount).into());
        }

        let mut locked_table = BalanceTable::from_root(store, &self.locked_table)
            .map_err(|e| e.downcast_wrap("failed to load locked table"))?;

        locked_table.must_subtract(addr, amount).map_err(|e| {
            e.downcast_default(ExitCode::USR_ILLEGAL_STATE, "subtract from locked table failed")
        })?;

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

        self.locked_table =
            locked_table.root().map_err(|e| e.downcast_wrap("failed to flush locked table"))?;

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

        let mut escrow_table = BalanceTable::from_root(store, &self.escrow_table).map_err(|e| {
            e.downcast_default(ExitCode::USR_ILLEGAL_STATE, "failed to load escrow table")
        })?;

        // Subtract from locked and escrow tables
        escrow_table
            .must_subtract(from_addr, amount)
            .map_err(|e| e.downcast_default(ExitCode::USR_ILLEGAL_STATE, "subtract from escrow"))?;

        self.unlock_balance(store, from_addr, amount, Reason::ClientStorageFee)
            .map_err(|e| e.downcast_default(ExitCode::USR_ILLEGAL_STATE, "subtract from locked"))?;

        // Add subtracted amount to the recipient
        escrow_table
            .add(to_addr, amount)
            .map_err(|e| e.downcast_default(ExitCode::USR_ILLEGAL_STATE, "add to escrow"))?;

        self.escrow_table = escrow_table.root().map_err(|e| {
            e.downcast_default(ExitCode::USR_ILLEGAL_STATE, "failed to flush escrow table")
        })?;

        Ok(())
    }

    fn slash_balance<BS>(
        &mut self,
        store: &BS,
        addr: &Address,
        amount: &TokenAmount,
        lock_reason: Reason,
    ) -> anyhow::Result<()>
    where
        BS: Blockstore,
    {
        if amount.is_negative() {
            return Err(actor_error!(illegal_state, "negative amount to slash: {}", amount).into());
        }

        let mut escrow_table = BalanceTable::from_root(store, &self.escrow_table)
            .map_err(|e| e.downcast_wrap("failed to load escrow table"))?;

        // Subtract from locked and escrow tables
        escrow_table.must_subtract(addr, amount).map_err(|e| {
            e.downcast_default(ExitCode::USR_ILLEGAL_STATE, "subtract from escrow failed")
        })?;

        self.escrow_table =
            escrow_table.root().map_err(|e| e.downcast_wrap("failed to flush escrow table"))?;

        self.unlock_balance(store, addr, amount, lock_reason)
    }
}
