// Copyright 2019-2022 ChainSafe Systems
// SPDX-License-Identifier: Apache-2.0, MIT

use crate::balance_table::BalanceTable;
use cid::Cid;
use fil_actors_runtime::ActorContext2;
use fil_actors_runtime::{
    actor_error, make_empty_map, runtime::Policy, ActorContext, ActorError, Array, Set, SetMultimap,
};
use fvm_ipld_blockstore::Blockstore;
use fvm_ipld_encoding::tuple::*;
use fvm_ipld_encoding::Cbor;
use fvm_shared::address::Address;
use fvm_shared::bigint::bigint_ser;
use fvm_shared::clock::{ChainEpoch, EPOCH_UNDEFINED};
use fvm_shared::deal::DealID;
use fvm_shared::econ::TokenAmount;
use fvm_shared::error::ExitCode;
use fvm_shared::HAMT_BIT_WIDTH;
use num_traits::{Signed, Zero};

use super::policy::*;
use super::types::*;
use super::{DealProposal, DealState};

/// Market actor state
#[derive(Clone, Default, Serialize_tuple, Deserialize_tuple)]
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
    #[serde(with = "bigint_ser")]
    pub total_client_locked_collateral: TokenAmount,
    /// Total Provider Collateral that is locked -> unlocked when deal is terminated
    #[serde(with = "bigint_ser")]
    pub total_provider_locked_collateral: TokenAmount,
    /// Total storage fee that is locked in escrow -> unlocked when payments are made
    #[serde(with = "bigint_ser")]
    pub total_client_storage_fee: TokenAmount,
}

impl State {
    pub fn new<BS: Blockstore>(store: &BS) -> Result<Self, ActorError> {
        let empty_proposals_array =
            Array::<(), BS>::new_with_bit_width(store, PROPOSALS_AMT_BITWIDTH)
                .flush()
                .context_code(
                    ExitCode::USR_ILLEGAL_STATE,
                    "Failed to create empty proposals array",
                )?;
        let empty_states_array = Array::<(), BS>::new_with_bit_width(store, STATES_AMT_BITWIDTH)
            .flush()
            .context_code(ExitCode::USR_ILLEGAL_STATE, "Failed to create empty states array")?;

        let empty_pending_proposals_map =
            make_empty_map::<_, ()>(store, HAMT_BIT_WIDTH).flush().context_code(
                ExitCode::USR_ILLEGAL_STATE,
                "Failed to create empty pending proposals map state",
            )?;
        let empty_balance_table = BalanceTable::new(store).root().context_code(
            ExitCode::USR_ILLEGAL_STATE,
            "Failed to create empty balance table map",
        )?;

        let empty_deal_ops_hamt = SetMultimap::new(store)
            .root()
            .context_code(ExitCode::USR_ILLEGAL_STATE, "Failed to create empty multiset")?;

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
        })
    }

    pub fn total_locked(&self) -> TokenAmount {
        &self.total_client_locked_collateral
            + &self.total_provider_locked_collateral
            + &self.total_client_storage_fee
    }

    pub(super) fn mutator<'bs, BS: Blockstore>(
        &mut self,
        store: &'bs BS,
    ) -> MarketStateMutation<'bs, '_, BS> {
        MarketStateMutation::new(self, store)
    }
}

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

impl Cbor for State {}

#[derive(Debug, PartialEq)]
pub(super) enum Permission {
    Invalid,
    ReadOnly,
    Write,
}

pub(super) enum Reason {
    ClientCollateral,
    ClientStorageFee,
    ProviderCollateral,
}

pub(super) struct MarketStateMutation<'bs, 's, BS> {
    pub(super) st: &'s mut State,
    pub(super) store: &'bs BS,

    pub(super) proposal_permit: Permission,
    pub(super) deal_proposals: Option<DealArray<'bs, BS>>,

    pub(super) state_permit: Permission,
    pub(super) deal_states: Option<DealMetaArray<'bs, BS>>,

    pub(super) escrow_permit: Permission,
    pub(super) escrow_table: Option<BalanceTable<'bs, BS>>,

    pub(super) pending_permit: Permission,
    pub(super) pending_deals: Option<Set<'bs, BS>>,

    pub(super) dpe_permit: Permission,
    pub(super) deals_by_epoch: Option<SetMultimap<'bs, BS>>,

    pub(super) locked_permit: Permission,
    pub(super) locked_table: Option<BalanceTable<'bs, BS>>,
    pub(super) total_client_locked_collateral: Option<TokenAmount>,
    pub(super) total_provider_locked_collateral: Option<TokenAmount>,
    pub(super) total_client_storage_fee: Option<TokenAmount>,

    pub(super) next_deal_id: DealID,
}

impl<'bs, 's, BS> MarketStateMutation<'bs, 's, BS>
where
    BS: Blockstore,
{
    pub(super) fn new(st: &'s mut State, store: &'bs BS) -> Self {
        Self {
            next_deal_id: st.next_id,
            st,
            store,
            proposal_permit: Permission::Invalid,
            deal_proposals: None,
            state_permit: Permission::Invalid,
            deal_states: None,
            escrow_permit: Permission::Invalid,
            escrow_table: None,
            pending_permit: Permission::Invalid,
            pending_deals: None,
            dpe_permit: Permission::Invalid,
            deals_by_epoch: None,
            locked_permit: Permission::Invalid,
            locked_table: None,
            total_client_locked_collateral: None,
            total_provider_locked_collateral: None,
            total_client_storage_fee: None,
        }
    }

    pub(super) fn build(&mut self) -> Result<&mut Self, ActorError> {
        if self.proposal_permit != Permission::Invalid {
            self.deal_proposals = Some(
                DealArray::load(&self.st.proposals, self.store)
                    .exit_code(ExitCode::USR_SERIALIZATION)?,
            );
        }

        if self.state_permit != Permission::Invalid {
            self.deal_states = Some(
                DealMetaArray::load(&self.st.states, self.store)
                    .exit_code(ExitCode::USR_SERIALIZATION)?,
            );
        }

        if self.locked_permit != Permission::Invalid {
            self.locked_table = Some(
                BalanceTable::from_root(self.store, &self.st.locked_table)
                    .exit_code(ExitCode::USR_SERIALIZATION)?,
            );
            self.total_client_locked_collateral =
                Some(self.st.total_client_locked_collateral.clone());
            self.total_client_storage_fee = Some(self.st.total_client_storage_fee.clone());
            self.total_provider_locked_collateral =
                Some(self.st.total_provider_locked_collateral.clone());
        }

        if self.escrow_permit != Permission::Invalid {
            self.escrow_table = Some(
                BalanceTable::from_root(self.store, &self.st.escrow_table)
                    .exit_code(ExitCode::USR_SERIALIZATION)?,
            );
        }

        if self.pending_permit != Permission::Invalid {
            self.pending_deals = Some(
                Set::from_root(self.store, &self.st.pending_proposals)
                    .exit_code(ExitCode::USR_SERIALIZATION)?,
            );
        }

        if self.dpe_permit != Permission::Invalid {
            self.deals_by_epoch = Some(
                SetMultimap::from_root(self.store, &self.st.deal_ops_by_epoch)
                    .exit_code(ExitCode::USR_SERIALIZATION)?,
            );
        }

        self.next_deal_id = self.st.next_id;

        Ok(self)
    }

    pub(super) fn with_deal_proposals(&mut self, permit: Permission) -> &mut Self {
        self.proposal_permit = permit;
        self
    }

    pub(super) fn with_deal_states(&mut self, permit: Permission) -> &mut Self {
        self.state_permit = permit;
        self
    }

    pub(super) fn with_escrow_table(&mut self, permit: Permission) -> &mut Self {
        self.escrow_permit = permit;
        self
    }

    pub(super) fn with_locked_table(&mut self, permit: Permission) -> &mut Self {
        self.locked_permit = permit;
        self
    }

    pub(super) fn with_pending_proposals(&mut self, permit: Permission) -> &mut Self {
        self.pending_permit = permit;
        self
    }

    pub(super) fn with_deals_by_epoch(&mut self, permit: Permission) -> &mut Self {
        self.dpe_permit = permit;
        self
    }

    pub(super) fn commit_state(&mut self) -> Result<(), ActorError> {
        if self.proposal_permit == Permission::Write {
            if let Some(s) = &mut self.deal_proposals {
                self.st.proposals = s
                    .flush()
                    .context_code(ExitCode::USR_ILLEGAL_STATE, "failed to flush deal proposals")?;
            }
        }

        if self.state_permit == Permission::Write {
            if let Some(s) = &mut self.deal_states {
                self.st.states = s
                    .flush()
                    .context_code(ExitCode::USR_ILLEGAL_STATE, "failed to flush deal states")?;
            }
        }

        if self.locked_permit == Permission::Write {
            if let Some(s) = &mut self.locked_table {
                self.st.locked_table = s
                    .root()
                    .context_code(ExitCode::USR_ILLEGAL_STATE, "failed to flush locked table")?;
            }
            if let Some(s) = &mut self.total_client_locked_collateral {
                self.st.total_client_locked_collateral = s.clone();
            }
            if let Some(s) = &mut self.total_provider_locked_collateral {
                self.st.total_provider_locked_collateral = s.clone();
            }
            if let Some(s) = &mut self.total_client_storage_fee {
                self.st.total_client_storage_fee = s.clone();
            }
        }

        if self.escrow_permit == Permission::Write {
            if let Some(s) = &mut self.escrow_table {
                self.st.escrow_table = s
                    .root()
                    .context_code(ExitCode::USR_ILLEGAL_STATE, "failed to flush escrow table")?;
            }
        }

        if self.pending_permit == Permission::Write {
            if let Some(s) = &mut self.pending_deals {
                self.st.pending_proposals = s
                    .root()
                    .context_code(ExitCode::USR_ILLEGAL_STATE, "failed to flush escrow table")?;
            }
        }

        if self.dpe_permit == Permission::Write {
            if let Some(s) = &mut self.deals_by_epoch {
                self.st.deal_ops_by_epoch = s
                    .root()
                    .context_code(ExitCode::USR_ILLEGAL_STATE, "failed to flush escrow table")?;
            }
        }

        self.st.next_id = self.next_deal_id;

        Ok(())
    }

    ////////////////////////////////////////////////////////////////////////////////
    // Deal state operations
    ////////////////////////////////////////////////////////////////////////////////
    #[allow(clippy::too_many_arguments)]
    pub(super) fn update_pending_deal_state(
        &mut self,
        policy: &Policy,
        state: &DealState,
        deal: &DealProposal,
        epoch: ChainEpoch,
    ) -> Result<(TokenAmount, ChainEpoch, bool), ActorError> {
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
        if total_payment > 0.into() {
            self.transfer_balance(&deal.client, &deal.provider, &total_payment)?;
        }

        if ever_slashed {
            // unlock client collateral and locked storage fee
            let payment_remaining = deal_get_payment_remaining(deal, state.slash_epoch)?;

            // Unlock remaining storage fee
            self.unlock_balance(&deal.client, &payment_remaining, Reason::ClientStorageFee)
                .context("failed to unlock remaining client storage fee")?;

            // Unlock client collateral
            self.unlock_balance(&deal.client, &deal.client_collateral, Reason::ClientCollateral)
                .context("failed to unlock client collateral")?;

            // slash provider collateral
            let slashed = deal.provider_collateral.clone();
            self.slash_balance(&deal.provider, &slashed, Reason::ProviderCollateral)
                .context("slashing balance")?;

            return Ok((slashed, EPOCH_UNDEFINED, true));
        }

        if epoch >= deal.end_epoch {
            self.process_deal_expired(deal, state)?;
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
    pub(super) fn process_deal_init_timed_out(
        &mut self,
        deal: &DealProposal,
    ) -> Result<TokenAmount, ActorError> {
        self.unlock_balance(&deal.client, &deal.total_storage_fee(), Reason::ClientStorageFee)
            .context("failure unlocking client storage fee")?;

        self.unlock_balance(&deal.client, &deal.client_collateral, Reason::ClientCollateral)
            .context("failure unlocking client collateral")?;

        let amount_slashed =
            collateral_penalty_for_deal_activation_missed(deal.provider_collateral.clone());
        let amount_remaining = deal.provider_balance_requirement() - &amount_slashed;

        self.slash_balance(&deal.provider, &amount_slashed, Reason::ProviderCollateral)
            .context("failed to slash balance")?;

        self.unlock_balance(&deal.provider, &amount_remaining, Reason::ProviderCollateral)
            .context("failed to unlock deal provider balance")?;

        Ok(amount_slashed)
    }

    /// Normal expiration. Unlock collaterals for both miner and client.
    fn process_deal_expired(
        &mut self,
        deal: &DealProposal,
        state: &DealState,
    ) -> Result<(), ActorError>
    where
        BS: Blockstore,
    {
        if state.sector_start_epoch == EPOCH_UNDEFINED {
            return Err(actor_error!(illegal_state, "start sector epoch undefined"));
        }

        self.unlock_balance(&deal.provider, &deal.provider_collateral, Reason::ProviderCollateral)
            .context("failed unlocking deal provider balance")?;

        self.unlock_balance(&deal.client, &deal.client_collateral, Reason::ClientCollateral)
            .context("failed unlocking deal client balance")?;

        Ok(())
    }

    pub(super) fn generate_storage_deal_id(&mut self) -> DealID {
        let ret = self.next_deal_id;
        self.next_deal_id += 1;
        ret
    }

    // Return true when the funds in escrow for the input address can cover an additional lockup of amountToLock
    pub(super) fn balance_covered(
        &self,
        addr: Address,
        amount_to_lock: &TokenAmount,
    ) -> Result<bool, ActorError> {
        let prev_locked = self
            .locked_table
            .as_ref()
            .unwrap()
            .get(&addr)
            .context_code(ExitCode::USR_ILLEGAL_STATE, "failed to get locked balance")?;
        let escrow_balance = self
            .escrow_table
            .as_ref()
            .unwrap()
            .get(&addr)
            .context_code(ExitCode::USR_ILLEGAL_STATE, "failed to get escrow balance")?;
        Ok((prev_locked + amount_to_lock) <= escrow_balance)
    }

    pub(super) fn maybe_lock_balance(
        &mut self,
        addr: &Address,
        amount: &TokenAmount,
    ) -> Result<(), ActorError> {
        if amount.is_negative() {
            return Err(actor_error!(illegal_state, "cannot lock negative amount {}", amount));
        }

        let prev_locked = self
            .locked_table
            .as_ref()
            .unwrap()
            .get(addr)
            .context_code(ExitCode::USR_ILLEGAL_STATE, "failed to get locked balance")?;

        let escrow_balance = self
            .escrow_table
            .as_ref()
            .unwrap()
            .get(addr)
            .context_code(ExitCode::USR_ILLEGAL_STATE, "failed to get escrow balance")?;

        if &prev_locked + amount > escrow_balance {
            return Err(actor_error!(insufficient_funds;
                    "not enough balance to lock for addr{}: \
                    escrow balance {} < prev locked {} + amount {}",
                    addr, escrow_balance, prev_locked, amount));
        }

        self.locked_table
            .as_mut()
            .unwrap()
            .add(addr, amount)
            .context_code(ExitCode::USR_ILLEGAL_STATE, "failed to add locked balance")?;
        Ok(())
    }

    pub(super) fn lock_client_and_provider_balances(
        &mut self,
        proposal: &DealProposal,
    ) -> Result<(), ActorError> {
        self.maybe_lock_balance(&proposal.client, &proposal.client_balance_requirement())
            .map_err(|e| e.wrap("failed to lock client funds"))?;

        self.maybe_lock_balance(&proposal.provider, &proposal.provider_collateral)
            .map_err(|e| e.wrap("failed to lock provider funds"))?;

        if let Some(v) = self.total_client_locked_collateral.as_mut() {
            *v += &proposal.client_collateral;
        }
        if let Some(v) = self.total_client_storage_fee.as_mut() {
            *v += proposal.total_storage_fee();
        }
        if let Some(v) = self.total_provider_locked_collateral.as_mut() {
            *v += &proposal.provider_collateral;
        }
        Ok(())
    }

    fn unlock_balance(
        &mut self,
        addr: &Address,
        amount: &TokenAmount,
        lock_reason: Reason,
    ) -> Result<(), ActorError> {
        if amount.is_negative() {
            return Err(actor_error!(illegal_state, "unlock negative amount: {}", amount));
        }
        self.locked_table.as_mut().unwrap().must_subtract(addr, amount)?;

        match lock_reason {
            Reason::ClientCollateral => self.total_client_locked_collateral.as_mut().map(|v| {
                *v -= amount;
            }),
            Reason::ClientStorageFee => self.total_client_storage_fee.as_mut().map(|v| {
                *v -= amount;
            }),
            Reason::ProviderCollateral => self.total_provider_locked_collateral.as_mut().map(|v| {
                *v -= amount;
            }),
        };

        Ok(())
    }

    /// move funds from locked in client to available in provider
    fn transfer_balance(
        &mut self,
        from_addr: &Address,
        to_addr: &Address,
        amount: &TokenAmount,
    ) -> Result<(), ActorError> {
        if amount.is_negative() {
            return Err(actor_error!(illegal_state, "transfer negative amount: {}", amount));
        }

        // Subtract from locked and escrow tables
        self.escrow_table
            .as_mut()
            .unwrap()
            .must_subtract(from_addr, amount)
            .context("subtract from escrow")?;

        self.unlock_balance(from_addr, amount, Reason::ClientStorageFee)
            .context("subtract from locked")?;

        // Add subtracted amount to the recipient
        self.escrow_table.as_mut().unwrap().add(to_addr, amount).context("add to escrow")?;

        Ok(())
    }

    fn slash_balance(
        &mut self,
        addr: &Address,
        amount: &TokenAmount,
        lock_reason: Reason,
    ) -> Result<(), ActorError> {
        if amount.is_negative() {
            return Err(actor_error!(illegal_state, "negative amount to slash: {}", amount));
        }

        // Subtract from locked and escrow tables
        self.escrow_table.as_mut().unwrap().must_subtract(addr, amount)?;
        self.unlock_balance(addr, amount, lock_reason)
    }
}
