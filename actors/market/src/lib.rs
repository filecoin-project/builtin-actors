// Copyright 2019-2022 ChainSafe Systems
// SPDX-License-Identifier: Apache-2.0, MIT

use cid::multihash::{Code, MultihashDigest, MultihashGeneric};
use cid::Cid;
use frc46_token::token::types::{TransferFromParams, TransferFromReturn};
use fvm_ipld_bitfield::BitField;
use fvm_ipld_blockstore::Blockstore;
use fvm_ipld_encoding::{Cbor, RawBytes};
use fvm_ipld_hamt::BytesKey;
use fvm_shared::address::Address;
use fvm_shared::bigint::BigInt;
use fvm_shared::clock::{ChainEpoch, QuantSpec, EPOCH_UNDEFINED};
use fvm_shared::deal::DealID;
use fvm_shared::econ::TokenAmount;
use fvm_shared::error::ExitCode;
use fvm_shared::piece::PieceInfo;
use fvm_shared::reward::ThisEpochRewardReturn;
use fvm_shared::sector::{RegisteredSealProof, SectorSize, StoragePower};
use fvm_shared::{ActorID, MethodNum, METHOD_CONSTRUCTOR, METHOD_SEND};
use integer_encoding::VarInt;
use log::info;
use num_derive::FromPrimitive;
use num_traits::{FromPrimitive, Zero};

use crate::balance_table::BalanceTable;
use fil_actors_runtime::cbor::{deserialize, serialize, serialize_vec};
use fil_actors_runtime::runtime::builtins::Type;
use fil_actors_runtime::runtime::{ActorCode, Policy, Runtime};
use fil_actors_runtime::{
    actor_error, cbor, restrict_internal_api, ActorContext, ActorDowncast, ActorError,
    AsActorError, BURNT_FUNDS_ACTOR_ADDR, CRON_ACTOR_ADDR, DATACAP_TOKEN_ACTOR_ADDR,
    REWARD_ACTOR_ADDR, STORAGE_POWER_ACTOR_ADDR, SYSTEM_ACTOR_ADDR, VERIFIED_REGISTRY_ACTOR_ADDR,
};

pub use self::deal::*;
use self::policy::*;
pub use self::state::*;
pub use self::types::*;

// exports for testing
pub mod balance_table;
#[doc(hidden)]
pub mod ext;
pub mod policy;
pub mod testing;

mod deal;
mod state;
mod types;

#[cfg(feature = "fil-actor")]
fil_actors_runtime::wasm_trampoline!(Actor);

pub const NO_ALLOCATION_ID: u64 = 0;

// An exit code indicating that information about a past deal is no longer available.
pub const EX_DEAL_EXPIRED: ExitCode = ExitCode::new(32);

/// Market actor methods available
#[derive(FromPrimitive)]
#[repr(u64)]
pub enum Method {
    Constructor = METHOD_CONSTRUCTOR,
    AddBalance = 2,
    WithdrawBalance = 3,
    PublishStorageDeals = 4,
    VerifyDealsForActivation = 5,
    ActivateDeals = 6,
    OnMinerSectorsTerminate = 7,
    ComputeDataCommitment = 8,
    CronTick = 9,
    // Method numbers derived from FRC-0042 standards
    AddBalanceExported = frc42_dispatch::method_hash!("AddBalance"),
    WithdrawBalanceExported = frc42_dispatch::method_hash!("WithdrawBalance"),
    PublishStorageDealsExported = frc42_dispatch::method_hash!("PublishStorageDeals"),
    GetBalanceExported = frc42_dispatch::method_hash!("GetBalance"),
    GetDealDataCommitmentExported = frc42_dispatch::method_hash!("GetDealDataCommitment"),
    GetDealClientExported = frc42_dispatch::method_hash!("GetDealClient"),
    GetDealProviderExported = frc42_dispatch::method_hash!("GetDealProvider"),
    GetDealLabelExported = frc42_dispatch::method_hash!("GetDealLabel"),
    GetDealTermExported = frc42_dispatch::method_hash!("GetDealTerm"),
    GetDealTotalPriceExported = frc42_dispatch::method_hash!("GetDealTotalPrice"),
    GetDealClientCollateralExported = frc42_dispatch::method_hash!("GetDealClientCollateral"),
    GetDealProviderCollateralExported = frc42_dispatch::method_hash!("GetDealProviderCollateral"),
    GetDealVerifiedExported = frc42_dispatch::method_hash!("GetDealVerified"),
    GetDealActivationExported = frc42_dispatch::method_hash!("GetDealActivation"),
}

/// Market Actor
pub struct Actor;

impl Actor {
    pub fn constructor(rt: &mut impl Runtime) -> Result<(), ActorError> {
        rt.validate_immediate_caller_is(std::iter::once(&SYSTEM_ACTOR_ADDR))?;

        let st = State::new(rt.store())?;
        rt.create(&st)?;
        Ok(())
    }

    /// Deposits the received value into the balance held in escrow.
    fn add_balance(rt: &mut impl Runtime, provider_or_client: Address) -> Result<(), ActorError> {
        let msg_value = rt.message().value_received();

        if msg_value <= TokenAmount::zero() {
            return Err(actor_error!(
                illegal_argument,
                "balance to add must be greater than zero was: {}",
                msg_value
            ));
        }

        rt.validate_immediate_caller_accept_any()?;

        let (nominal, _, _) = escrow_address(rt, &provider_or_client)?;

        rt.transaction(|st: &mut State, rt| {
            st.add_balance_to_escrow_table(rt.store(), &nominal, &msg_value)?;
            Ok(())
        })?;

        Ok(())
    }

    /// Attempt to withdraw the specified amount from the balance held in escrow.
    /// If less than the specified amount is available, yields the entire available balance.
    fn withdraw_balance(
        rt: &mut impl Runtime,
        params: WithdrawBalanceParams,
    ) -> Result<WithdrawBalanceReturn, ActorError> {
        if params.amount < TokenAmount::zero() {
            return Err(actor_error!(illegal_argument, "negative amount: {}", params.amount));
        }

        let (nominal, recipient, approved) = escrow_address(rt, &params.provider_or_client)?;
        // for providers -> only corresponding owner or worker can withdraw
        // for clients -> only the client i.e the recipient can withdraw
        rt.validate_immediate_caller_is(&approved)?;

        let amount_extracted = rt.transaction(|st: &mut State, rt| {
            let ex = st.withdraw_balance_from_escrow_table(rt.store(), &nominal, &params.amount)?;

            Ok(ex)
        })?;

        rt.send(&recipient, METHOD_SEND, RawBytes::default(), amount_extracted.clone())?;

        Ok(WithdrawBalanceReturn { amount_withdrawn: amount_extracted })
    }

    /// Returns the escrow balance and locked amount for an address.
    fn get_balance(
        rt: &mut impl Runtime,
        account: Address,
    ) -> Result<GetBalanceReturn, ActorError> {
        rt.validate_immediate_caller_accept_any()?;
        let nominal = rt.resolve_address(&account).ok_or_else(|| {
            actor_error!(illegal_argument, "failed to resolve address {}", account)
        })?;
        let account = Address::new_id(nominal);

        let store = rt.store();
        let st: State = rt.state()?;
        let balances = BalanceTable::from_root(store, &st.escrow_table)
            .context_code(ExitCode::USR_ILLEGAL_STATE, "failed to load escrow table")?;
        let locks = BalanceTable::from_root(store, &st.locked_table)
            .context_code(ExitCode::USR_ILLEGAL_STATE, "failed to load locked table")?;
        let balance = balances
            .get(&account)
            .context_code(ExitCode::USR_ILLEGAL_STATE, "failed to get escrow balance")?;
        let locked = locks
            .get(&account)
            .context_code(ExitCode::USR_ILLEGAL_STATE, "failed to get locked balance")?;

        Ok(GetBalanceReturn { balance, locked })
    }

    /// Publish a new set of storage deals (not yet included in a sector).
    fn publish_storage_deals(
        rt: &mut impl Runtime,
        params: PublishStorageDealsParams,
    ) -> Result<PublishStorageDealsReturn, ActorError> {
        rt.validate_immediate_caller_accept_any()?;
        if params.deals.is_empty() {
            return Err(actor_error!(illegal_argument, "Empty deals parameter"));
        }

        // All deals should have the same provider so get worker once
        let provider_raw = params.deals[0].proposal.provider;
        let provider_id = rt.resolve_address(&provider_raw).ok_or_else(|| {
            actor_error!(not_found, "failed to resolve provider address {}", provider_raw)
        })?;

        let code_id = rt
            .get_actor_code_cid(&provider_id)
            .ok_or_else(|| actor_error!(not_found, "no code ID for address {}", provider_id))?;

        if rt.resolve_builtin_actor_type(&code_id) != Some(Type::Miner) {
            return Err(actor_error!(
                illegal_argument,
                "deal provider is not a storage miner actor"
            ));
        }

        let (_, worker, controllers) = request_miner_control_addrs(rt, provider_id)?;
        let caller = rt.message().caller();
        let mut caller_ok = caller == worker;
        for controller in controllers.iter() {
            if caller_ok {
                break;
            }
            caller_ok = caller == *controller;
        }
        if !caller_ok {
            return Err(actor_error!(
                forbidden,
                "caller {} is not worker or control address of provider {}",
                caller,
                provider_id
            ));
        }

        let baseline_power = request_current_baseline_power(rt)?;
        let (network_raw_power, _) = request_current_network_power(rt)?;

        struct ValidDeal {
            proposal: DealProposal,
            cid: Cid,
            allocation: AllocationID,
        }

        // Deals that passed validation.
        let mut valid_deals: Vec<ValidDeal> = Vec::with_capacity(params.deals.len());
        // CIDs of valid proposals.
        let mut proposal_cid_lookup = BTreeSet::new();
        let mut total_client_lockup: BTreeMap<ActorID, TokenAmount> = BTreeMap::new();
        let mut total_provider_lockup = TokenAmount::zero();

        let mut valid_input_bf = BitField::default();
        let curr_epoch = rt.curr_epoch();

        let state: State = rt.state()?;

        for (di, mut deal) in params.deals.into_iter().enumerate() {
            // drop malformed deals
            if let Err(e) = validate_deal(rt, &deal, &network_raw_power, &baseline_power) {
                info!("invalid deal {}: {}", di, e);
                continue;
            }

            if deal.proposal.provider != Address::new_id(provider_id)
                && deal.proposal.provider != provider_raw
            {
                info!(
                    "invalid deal {}: cannot publish deals from multiple providers in one batch",
                    di
                );
                continue;
            }
            let client_id = match rt.resolve_address(&deal.proposal.client) {
                Some(client) => client,
                _ => {
                    info!(
                        "invalid deal {}: failed to resolve proposal.client address {} for deal",
                        di, deal.proposal.client
                    );
                    continue;
                }
            };

            // drop deals with insufficient lock up to cover costs
            let mut client_lockup =
                total_client_lockup.get(&client_id).cloned().unwrap_or_default();
            client_lockup += deal.proposal.client_balance_requirement();

            let client_balance_ok =
                state.balance_covered(rt.store(), Address::new_id(client_id), &client_lockup)?;

            if !client_balance_ok {
                info!("invalid deal: {}: insufficient client funds to cover proposal cost", di);
                continue;
            }

            let mut provider_lockup = total_provider_lockup.clone();
            provider_lockup += &deal.proposal.provider_collateral;
            let provider_balance_ok = state.balance_covered(
                rt.store(),
                Address::new_id(provider_id),
                &provider_lockup,
            )?;

            if !provider_balance_ok {
                info!("invalid deal: {}: insufficient provider funds to cover proposal cost", di);
                continue;
            }

            // drop duplicate deals
            // Normalise provider and client addresses in the proposal stored on chain.
            // Must happen after signature verification and before taking cid.
            deal.proposal.provider = Address::new_id(provider_id);
            deal.proposal.client = Address::new_id(client_id);
            let pcid = rt_deal_cid(rt, &deal.proposal).map_err(
                |e| actor_error!(illegal_argument; "failed to take cid of proposal {}: {}", di, e),
            )?;

            // check proposalCids for duplication within message batch
            // check state PendingProposals for duplication across messages
            let duplicate_in_state = state.has_pending_deal(rt.store(), pcid)?;

            let duplicate_in_message = proposal_cid_lookup.contains(&pcid);
            if duplicate_in_state || duplicate_in_message {
                info!("invalid deal {}: cannot publish duplicate deal proposal", di);
                continue;
            }

            // For verified deals, transfer datacap tokens from the client
            // to the verified registry actor along with a specification for the allocation.
            // Drop deal if the transfer fails.
            // This could be done in a batch, but one-at-a-time allows dropping of only
            // some deals if the client's balance is insufficient, rather than dropping them all.
            // An alternative could first fetch the available balance/allowance, and then make
            // a batch transfer for an amount known to be available.
            // https://github.com/filecoin-project/builtin-actors/issues/662
            let allocation_id = if deal.proposal.verified_deal {
                let params = datacap_transfer_request(
                    &Address::new_id(client_id),
                    vec![alloc_request_for_deal(&deal, rt.policy(), curr_epoch)],
                )?;
                let alloc_ids = rt
                    .send(
                        &DATACAP_TOKEN_ACTOR_ADDR,
                        ext::datacap::TRANSFER_FROM_METHOD as u64,
                        serialize(&params, "transfer parameters")?,
                        TokenAmount::zero(),
                    )
                    .and_then(|ret| datacap_transfer_response(&ret));
                match alloc_ids {
                    Ok(ids) => {
                        // Note: when changing this to do anything other than expect complete success,
                        // inspect the BatchReturn values to determine which deals succeeded and which failed.
                        if ids.len() != 1 {
                            return Err(actor_error!(
                                unspecified,
                                "expected 1 allocation ID, got {:?}",
                                ids
                            ));
                        }
                        ids[0]
                    }
                    Err(e) => {
                        info!(
                            "invalid deal {}: failed to allocate datacap for verified deal: {}",
                            di, e
                        );
                        continue;
                    }
                }
            } else {
                NO_ALLOCATION_ID
            };

            total_provider_lockup = provider_lockup;
            total_client_lockup.insert(client_id, client_lockup);
            proposal_cid_lookup.insert(pcid);
            valid_deals.push(ValidDeal {
                proposal: deal.proposal,
                cid: pcid,
                allocation: allocation_id,
            });
            valid_input_bf.set(di as u64)
        }

        let valid_deal_count = valid_input_bf.len();
        if valid_deal_count != valid_deals.len() as u64 {
            return Err(actor_error!(
                illegal_state,
                "{} valid deals but valid_deal_count {}",
                valid_deals.len(),
                valid_deal_count
            ));
        }
        if valid_deal_count == 0 {
            return Err(actor_error!(illegal_argument, "All deal proposals invalid"));
        }

        let mut new_deal_ids = Vec::with_capacity(valid_deals.len());
        rt.transaction(|st: &mut State, rt| {
            let mut pending_deals: Vec<Cid> = vec![];
            let mut deal_proposals: Vec<(DealID, DealProposal)> = vec![];
            let mut deals_by_epoch: Vec<(ChainEpoch, DealID)> = vec![];
            let mut pending_deal_allocation_ids: Vec<(BytesKey, AllocationID)> = vec![];

            // All storage dealProposals will be added in an atomic transaction; this operation will be unrolled if any of them fails.
            // This should only fail on programmer error because all expected invalid conditions should be filtered in the first set of checks.
            for valid_deal in valid_deals.iter() {
                st.lock_client_and_provider_balances(rt.store(), &valid_deal.proposal)?;

                // Store the proposal CID in pending deals set.
                pending_deals.push(valid_deal.cid);

                // Allocate a deal ID and store the proposal in the proposals AMT.
                let deal_id = st.generate_storage_deal_id();
                deal_proposals.push((deal_id, valid_deal.proposal.clone()));

                // Store verified allocation (if any) in the pending allocation IDs map.
                // It will be removed when the deal is activated or expires.
                if valid_deal.allocation != NO_ALLOCATION_ID {
                    pending_deal_allocation_ids.push((deal_id_key(deal_id), valid_deal.allocation));
                }

                // Randomize the first epoch for when the deal will be processed so an attacker isn't able to
                // schedule too many deals for the same tick.
                deals_by_epoch.push((
                    gen_rand_next_epoch(rt.policy(), valid_deal.proposal.start_epoch, deal_id),
                    deal_id,
                ));

                new_deal_ids.push(deal_id);
            }

            st.put_pending_deals(rt.store(), &pending_deals)?;

            st.put_deal_proposals(rt.store(), &deal_proposals)?;

            st.put_pending_deal_allocation_ids(rt.store(), &pending_deal_allocation_ids)?;

            st.put_deals_by_epoch(rt.store(), &deals_by_epoch)?;

            Ok(())
        })?;

        Ok(PublishStorageDealsReturn { ids: new_deal_ids, valid_deals: valid_input_bf })
    }

    /// Verify that a given set of storage deals is valid for a sector currently being PreCommitted
    /// and return UnsealedCID for the set of deals.
    fn verify_deals_for_activation(
        rt: &mut impl Runtime,
        params: VerifyDealsForActivationParams,
    ) -> Result<VerifyDealsForActivationReturn, ActorError> {
        rt.validate_immediate_caller_type(std::iter::once(&Type::Miner))?;
        let miner_addr = rt.message().caller();
        let curr_epoch = rt.curr_epoch();

        let st: State = rt.state()?;
        let proposals = st.get_proposal_array(rt.store())?;

        let mut sectors_data = Vec::with_capacity(params.sectors.len());
        for sector in params.sectors.iter() {
            let sector_size = sector
                .sector_type
                .sector_size()
                .map_err(|e| actor_error!(illegal_argument, "sector size unknown: {}", e))?;
            validate_and_return_deal_space(
                &proposals,
                &sector.deal_ids,
                &miner_addr,
                sector.sector_expiry,
                curr_epoch,
                Some(sector_size),
            )
            .context("failed to validate deal proposals for activation")?;

            let commd = if sector.deal_ids.is_empty() {
                None
            } else {
                Some(compute_data_commitment(rt, &proposals, sector.sector_type, &sector.deal_ids)?)
            };

            sectors_data.push(SectorDealData { commd });
        }

        Ok(VerifyDealsForActivationReturn { sectors: sectors_data })
    }
    /// Activate a set of deals, returning the combined deal space and extra info for verified deals.
    fn activate_deals(
        rt: &mut impl Runtime,
        params: ActivateDealsParams,
    ) -> Result<ActivateDealsResult, ActorError> {
        rt.validate_immediate_caller_type(std::iter::once(&Type::Miner))?;
        let miner_addr = rt.message().caller();
        let curr_epoch = rt.curr_epoch();

        let st: State = rt.state()?;
        let proposals = st.get_proposal_array(rt.store())?;

        let deal_spaces = {
            validate_and_return_deal_space(
                &proposals,
                &params.deal_ids,
                &miner_addr,
                params.sector_expiry,
                curr_epoch,
                None,
            )
            .context("failed to validate deal proposals for activation")?
        };

        // Update deal states
        let mut verified_infos = Vec::new();
        rt.transaction(|st: &mut State, rt| {
            let mut deal_states: Vec<(DealID, DealState)> = vec![];

            for deal_id in params.deal_ids {
                // This construction could be replaced with a single "update deal state"
                // state method, possibly batched over all deal ids at once.
                let s = st.find_deal_state(rt.store(), deal_id)?;

                if s.is_some() {
                    return Err(actor_error!(
                        illegal_argument,
                        "deal {} already activated",
                        deal_id
                    ));
                }

                let proposal = st
                    .find_proposal(rt.store(), deal_id)?
                    .ok_or_else(|| actor_error!(not_found, "no such deal_id: {}", deal_id))?;

                let propc = rt_deal_cid(rt, &proposal)?;

                // Confirm the deal is in the pending proposals queue.
                // It will be removed from this queue later, during cron.
                let has = st.has_pending_deal(rt.store(), propc)?;

                if !has {
                    return Err(actor_error!(
                        illegal_state,
                        "tried to activate deal that was not in the pending set ({})",
                        propc
                    ));
                }

                // Extract and remove any verified allocation ID for the pending deal.
                let allocation = st
                    .remove_pending_deal_allocation_id(rt.store(), &deal_id_key(deal_id))?
                    .unwrap_or((BytesKey(vec![]), NO_ALLOCATION_ID))
                    .1;

                if allocation != NO_ALLOCATION_ID {
                    verified_infos.push(VerifiedDealInfo {
                        client: proposal.client.id().unwrap(),
                        allocation_id: allocation,
                        data: proposal.piece_cid,
                        size: proposal.piece_size,
                    })
                }

                deal_states.push((
                    deal_id,
                    DealState {
                        sector_start_epoch: curr_epoch,
                        last_updated_epoch: EPOCH_UNDEFINED,
                        slash_epoch: EPOCH_UNDEFINED,
                        verified_claim: allocation,
                    },
                ));
            }

            st.put_deal_states(rt.store(), &deal_states)?;

            Ok(())
        })?;

        Ok(ActivateDealsResult { nonverified_deal_space: deal_spaces.deal_space, verified_infos })
    }

    /// Terminate a set of deals in response to their containing sector being terminated.
    /// Slash provider collateral, refund client collateral, and refund partial unpaid escrow
    /// amount to client.
    fn on_miner_sectors_terminate(
        rt: &mut impl Runtime,
        params: OnMinerSectorsTerminateParams,
    ) -> Result<(), ActorError> {
        rt.validate_immediate_caller_type(std::iter::once(&Type::Miner))?;
        let miner_addr = rt.message().caller();

        rt.transaction(|st: &mut State, rt| {
            let mut deal_states: Vec<(DealID, DealState)> = vec![];

            for id in params.deal_ids {
                let deal = st.find_proposal(rt.store(), id)?;

                // The deal may have expired and been deleted before the sector is terminated.
                // Nothing to do, but continue execution for the other deals.
                if deal.is_none() {
                    info!("couldn't find deal {}", id);
                    continue;
                }
                let deal = deal.unwrap();

                if deal.provider != miner_addr {
                    return Err(actor_error!(
                        illegal_state,
                        "caller {} is not the provider {} of deal {}",
                        miner_addr,
                        deal.provider,
                        id
                    ));
                }

                // do not slash expired deals
                if deal.end_epoch <= params.epoch {
                    info!("deal {} expired, not slashing", id);
                    continue;
                }

                let mut state: DealState = st
                    .find_deal_state(rt.store(), id)?
                    // A deal with a proposal but no state is not activated, but then it should not be
                    // part of a sector that is terminating.
                    .ok_or_else(|| actor_error!(illegal_argument, "no state for deal {}", id))?;

                // If a deal is already slashed, don't need to do anything
                if state.slash_epoch != EPOCH_UNDEFINED {
                    info!("deal {}, already slashed", id);
                    continue;
                }

                // mark the deal for slashing here. Actual releasing of locked funds for the client
                // and slashing of provider collateral happens in cron_tick.
                state.slash_epoch = params.epoch;

                deal_states.push((id, state));
            }

            st.put_deal_states(rt.store(), &deal_states)?;
            Ok(())
        })?;
        Ok(())
    }

    fn compute_data_commitment(
        rt: &mut impl Runtime,
        params: ComputeDataCommitmentParams,
    ) -> Result<ComputeDataCommitmentReturn, ActorError> {
        rt.validate_immediate_caller_type(std::iter::once(&Type::Miner))?;

        let st: State = rt.state()?;
        let proposals = st.get_proposal_array(rt.store())?;

        let mut commds = Vec::with_capacity(params.inputs.len());
        for comm_input in params.inputs.iter() {
            commds.push(compute_data_commitment(
                rt,
                &proposals,
                comm_input.sector_type,
                &comm_input.deal_ids,
            )?);
        }

        Ok(ComputeDataCommitmentReturn { commds })
    }

    fn cron_tick(rt: &mut impl Runtime) -> Result<(), ActorError> {
        rt.validate_immediate_caller_is(std::iter::once(&CRON_ACTOR_ADDR))?;

        let mut amount_slashed = TokenAmount::zero();
        let curr_epoch = rt.curr_epoch();

        rt.transaction(|st: &mut State, rt| {
            let last_cron = st.last_cron;
            let mut updates_needed: BTreeMap<ChainEpoch, Vec<DealID>> = BTreeMap::new();
            let mut rm_cron_id: Vec<ChainEpoch> = vec![];

            for i in (last_cron + 1)..=rt.curr_epoch() {
                let deal_ids = st.get_deals_for_epoch(rt.store(), i)?;

                for deal_id in deal_ids {
                    let deal = st.find_proposal(rt.store(), deal_id)?.ok_or_else(|| {
                        actor_error!(not_found, "proposal doesn't exist ({})", deal_id)
                    })?;

                    let dcid = rt_deal_cid(rt, &deal)?;

                    let state = st.find_deal_state(rt.store(), deal_id)?;

                    // deal has been published but not activated yet -> terminate it
                    // as it has timed out
                    if state.is_none() {
                        // Not yet appeared in proven sector; check for timeout.
                        if curr_epoch < deal.start_epoch {
                            return Err(actor_error!(
                                illegal_state,
                                "deal {} processed before start epoch {}",
                                deal_id,
                                deal.start_epoch
                            ));
                        }

                        let slashed = st.process_deal_init_timed_out(rt.store(), &deal)?;
                        if !slashed.is_zero() {
                            amount_slashed += slashed;
                        }

                        // Delete the proposal (but not state, which doesn't exist).
                        let deleted = st.remove_proposal(rt.store(), deal_id)?;

                        if deleted.is_none() {
                            return Err(actor_error!(
                                illegal_state,
                                format!(
                                    "failed to delete deal {} proposal {}: does not exist",
                                    deal_id, dcid
                                )
                            ));
                        }

                        // Delete pending deal CID
                        st.remove_pending_deal(rt.store(), dcid)?.ok_or_else(|| {
                            actor_error!(
                                illegal_state,
                                "failed to delete pending deals: does not exist"
                            )
                        })?;

                        // Delete pending deal allocation id (if present).
                        st.remove_pending_deal_allocation_id(rt.store(), &deal_id_key(deal_id))?;

                        continue;
                    }
                    let mut state = state.unwrap();

                    if state.last_updated_epoch == EPOCH_UNDEFINED {
                        st.remove_pending_deal(rt.store(), dcid)?.ok_or_else(|| {
                            actor_error!(
                                illegal_state,
                                "failed to delete pending proposal: does not exist"
                            )
                        })?;
                    }

                    let (slash_amount, next_epoch, remove_deal) = st.put_pending_deal_state(
                        rt.store(),
                        rt.policy(),
                        &state,
                        &deal,
                        curr_epoch,
                    )?;

                    if slash_amount.is_negative() {
                        return Err(actor_error!(
                            illegal_state,
                            format!(
                                "computed negative slash amount {} for deal {}",
                                slash_amount, deal_id
                            )
                        ));
                    }

                    if remove_deal {
                        if next_epoch != EPOCH_UNDEFINED {
                            return Err(actor_error!(
                                illegal_state,
                                format!(
                                    "removed deal {} should have no scheduled epoch (got {})",
                                    deal_id, next_epoch
                                )
                            ));
                        }

                        amount_slashed += slash_amount;

                        // Delete proposal and state simultaneously.
                        let deleted = st.remove_deal_state(rt.store(), deal_id)?;

                        if deleted.is_none() {
                            return Err(actor_error!(
                                illegal_state,
                                "failed to delete deal state: does not exist"
                            ));
                        }

                        let deleted = st.remove_proposal(rt.store(), deal_id)?;

                        if deleted.is_none() {
                            return Err(actor_error!(
                                illegal_state,
                                "failed to delete deal proposal: does not exist"
                            ));
                        }
                    } else {
                        if next_epoch <= rt.curr_epoch() {
                            return Err(actor_error!(
                                illegal_state,
                                "continuing deal {} next epoch {} should be in the future",
                                deal_id,
                                next_epoch
                            ));
                        }
                        if !slash_amount.is_zero() {
                            return Err(actor_error!(
                                illegal_state,
                                "continuing deal {} should not be slashed",
                                deal_id
                            ));
                        }

                        state.last_updated_epoch = curr_epoch;
                        st.put_deal_states(rt.store(), &[(deal_id, state)])?;

                        if let Some(ev) = updates_needed.get_mut(&next_epoch) {
                            ev.push(deal_id);
                        } else {
                            updates_needed.insert(next_epoch, vec![deal_id]);
                        }
                    }
                }
                rm_cron_id.push(i);
            }

            st.remove_deals_by_epoch(rt.store(), &rm_cron_id)?;

            // updates_needed is already sorted by epoch.
            st.put_batch_deals_by_epoch(rt.store(), &updates_needed)?;

            st.last_cron = rt.curr_epoch();

            Ok(())
        })?;

        if !amount_slashed.is_zero() {
            rt.send(&BURNT_FUNDS_ACTOR_ADDR, METHOD_SEND, RawBytes::default(), amount_slashed)?;
        }
        Ok(())
    }

    /// Returns the data commitment and size of a deal proposal.
    /// This will be available after the deal is published (whether or not is is activated)
    /// and up until some undefined period after it is terminated.
    fn get_deal_data_commitment(
        rt: &mut impl Runtime,
        params: GetDealDataCommitmentParams,
    ) -> Result<GetDealDataCommitmentReturn, ActorError> {
        rt.validate_immediate_caller_accept_any()?;
        let found = rt.state::<State>()?.get_proposal(rt.store(), params.id)?;
        Ok(GetDealDataCommitmentReturn { data: found.piece_cid, size: found.piece_size })
    }

    /// Returns the client of a deal proposal.
    fn get_deal_client(
        rt: &mut impl Runtime,
        params: GetDealClientParams,
    ) -> Result<GetDealClientReturn, ActorError> {
        rt.validate_immediate_caller_accept_any()?;
        let found = rt.state::<State>()?.get_proposal(rt.store(), params.id)?;
        Ok(GetDealClientReturn { client: found.client.id().unwrap() })
    }

    /// Returns the provider of a deal proposal.
    fn get_deal_provider(
        rt: &mut impl Runtime,
        params: GetDealProviderParams,
    ) -> Result<GetDealProviderReturn, ActorError> {
        rt.validate_immediate_caller_accept_any()?;
        let found = rt.state::<State>()?.get_proposal(rt.store(), params.id)?;
        Ok(GetDealProviderReturn { provider: found.provider.id().unwrap() })
    }

    /// Returns the label of a deal proposal.
    fn get_deal_label(
        rt: &mut impl Runtime,
        params: GetDealLabelParams,
    ) -> Result<GetDealLabelReturn, ActorError> {
        rt.validate_immediate_caller_accept_any()?;
        let found = rt.state::<State>()?.get_proposal(rt.store(), params.id)?;
        Ok(GetDealLabelReturn { label: found.label })
    }

    /// Returns the start epoch and duration (in epochs) of a deal proposal.
    fn get_deal_term(
        rt: &mut impl Runtime,
        params: GetDealTermParams,
    ) -> Result<GetDealTermReturn, ActorError> {
        rt.validate_immediate_caller_accept_any()?;
        let found = rt.state::<State>()?.get_proposal(rt.store(), params.id)?;
        Ok(GetDealTermReturn { start: found.start_epoch, duration: found.duration() })
    }

    /// Returns the total price that will be paid from the client to the provider for this deal.
    fn get_deal_total_price(
        rt: &mut impl Runtime,
        params: GetDealTotalPriceParams,
    ) -> Result<GetDealTotalPriceReturn, ActorError> {
        rt.validate_immediate_caller_accept_any()?;
        let found = rt.state::<State>()?.get_proposal(rt.store(), params.id)?;
        Ok(GetDealTotalPriceReturn { total_price: found.total_storage_fee() })
    }

    /// Returns the client collateral requirement for a deal proposal.
    fn get_deal_client_collateral(
        rt: &mut impl Runtime,
        params: GetDealClientCollateralParams,
    ) -> Result<GetDealClientCollateralReturn, ActorError> {
        rt.validate_immediate_caller_accept_any()?;
        let found = rt.state::<State>()?.get_proposal(rt.store(), params.id)?;
        Ok(GetDealClientCollateralReturn { collateral: found.client_collateral })
    }

    /// Returns the provider collateral requirement for a deal proposal.
    fn get_deal_provider_collateral(
        rt: &mut impl Runtime,
        params: GetDealProviderCollateralParams,
    ) -> Result<GetDealProviderCollateralReturn, ActorError> {
        rt.validate_immediate_caller_accept_any()?;
        let found = rt.state::<State>()?.get_proposal(rt.store(), params.id)?;
        Ok(GetDealProviderCollateralReturn { collateral: found.provider_collateral })
    }

    /// Returns the verified flag for a deal proposal.
    /// Note that the source of truth for verified allocations and claims is
    /// the verified registry actor.
    fn get_deal_verified(
        rt: &mut impl Runtime,
        params: GetDealVerifiedParams,
    ) -> Result<GetDealVerifiedReturn, ActorError> {
        rt.validate_immediate_caller_accept_any()?;
        let found = rt.state::<State>()?.get_proposal(rt.store(), params.id)?;
        Ok(GetDealVerifiedReturn { verified: found.verified_deal })
    }

    /// Fetches activation state for a deal.
    /// This will be available from when the proposal is published until an undefined period after
    /// the deal finishes (either normally or by termination).
    /// Returns USR_NOT_FOUND if the deal doesn't exist (yet), or EX_DEAL_EXPIRED if the deal
    /// has been removed from state.
    fn get_deal_activation(
        rt: &mut impl Runtime,
        params: GetDealActivationParams,
    ) -> Result<GetDealActivationReturn, ActorError> {
        rt.validate_immediate_caller_accept_any()?;
        let st = rt.state::<State>()?;
        let found = st.find_deal_state(rt.store(), params.id)?;
        match found {
            Some(state) => Ok(GetDealActivationReturn {
                // If we have state, the deal has been activated.
                // It may also have completed normally, or been terminated,
                // but not yet been cleaned up.
                activated: state.sector_start_epoch,
                terminated: state.slash_epoch,
            }),
            None => {
                // State::get_proposal will fail with USR_NOT_FOUND in either case.
                let maybe_proposal = st.find_proposal(rt.store(), params.id)?;
                match maybe_proposal {
                    Some(_) => Ok(GetDealActivationReturn {
                        // The proposal has been published, but not activated.
                        activated: EPOCH_UNDEFINED,
                        terminated: EPOCH_UNDEFINED,
                    }),
                    None => {
                        if params.id < st.next_id {
                            // If the deal ID has been used, it must have been cleaned up.
                            Err(ActorError::unchecked(
                                EX_DEAL_EXPIRED,
                                format!("deal {} expired", params.id),
                            ))
                        } else {
                            // We can't distinguish between failing to activate, or having been
                            // cleaned up after completion/termination.
                            Err(ActorError::not_found(format!("no such deal {}", params.id)))
                        }
                    }
                }
            }
        }
    }
}

fn compute_data_commitment<BS: Blockstore>(
    rt: &impl Runtime,
    proposals: &DealArray<BS>,
    sector_type: RegisteredSealProof,
    deal_ids: &[DealID],
) -> Result<Cid, ActorError> {
    let mut pieces = Vec::with_capacity(deal_ids.len());

    for deal_id in deal_ids {
        let deal = proposals
            .get(*deal_id)
            .map_err(|e| {
                e.downcast_default(
                    ExitCode::USR_ILLEGAL_STATE,
                    format!("failed to get deal_id ({})", deal_id),
                )
            })?
            .ok_or_else(|| actor_error!(not_found, "proposal doesn't exist ({})", deal_id))?;

        pieces.push(PieceInfo { cid: deal.piece_cid, size: deal.piece_size });
    }
    rt.compute_unsealed_sector_cid(sector_type, &pieces).map_err(|e| {
        e.downcast_default(ExitCode::USR_ILLEGAL_ARGUMENT, "failed to compute unsealed sector CID")
    })
}

pub fn validate_and_return_deal_space<BS: Blockstore>(
    proposals: &DealArray<BS>,
    deal_ids: &[DealID],
    miner_addr: &Address,
    sector_expiry: ChainEpoch,
    sector_activation: ChainEpoch,
    sector_size: Option<SectorSize>,
) -> Result<DealSpaces, ActorError> {
    let mut seen_deal_ids = BTreeSet::new();
    let mut deal_space = BigInt::zero();
    let mut verified_deal_space = BigInt::zero();

    for deal_id in deal_ids {
        if !seen_deal_ids.insert(deal_id) {
            return Err(actor_error!(
                illegal_argument,
                "deal id {} present multiple times",
                deal_id
            ));
        }

        let proposal = proposals
            .get(*deal_id)
            .context_code(ExitCode::USR_ILLEGAL_STATE, "failed to load deal")?
            .ok_or_else(|| actor_error!(not_found, "no such deal {}", deal_id))?;

        validate_deal_can_activate(proposal, miner_addr, sector_expiry, sector_activation)
            .with_context(|| format!("cannot activate deal {}", deal_id))?;

        if proposal.verified_deal {
            verified_deal_space += proposal.piece_size.0;
        } else {
            deal_space += proposal.piece_size.0;
        }
    }
    if let Some(sector_size) = sector_size {
        let total_deal_space = deal_space.clone() + verified_deal_space.clone();
        if total_deal_space > BigInt::from(sector_size as u64) {
            return Err(actor_error!(
                illegal_argument,
                "deals too large to fit in sector {} > {}",
                total_deal_space,
                sector_size
            ));
        }
    }

    Ok(DealSpaces { deal_space, verified_deal_space })
}

fn alloc_request_for_deal(
    deal: &ClientDealProposal,
    policy: &Policy,
    curr_epoch: ChainEpoch,
) -> ext::verifreg::AllocationRequest {
    let alloc_term_min = deal.proposal.end_epoch - deal.proposal.start_epoch;
    let alloc_term_max = min(
        alloc_term_min + policy.market_default_allocation_term_buffer,
        policy.maximum_verified_allocation_term,
    );
    let alloc_expiration =
        min(deal.proposal.start_epoch, curr_epoch + policy.maximum_verified_allocation_expiration);
    ext::verifreg::AllocationRequest {
        provider: deal.proposal.provider,
        data: deal.proposal.piece_cid,
        size: deal.proposal.piece_size,
        term_min: alloc_term_min,
        term_max: alloc_term_max,
        expiration: alloc_expiration,
    }
}

// Builds TransferFromParams for a transfer of datacap for specified allocations.
fn datacap_transfer_request(
    client: &Address,
    alloc_reqs: Vec<AllocationRequest>,
) -> Result<TransferFromParams, ActorError> {
    let datacap_required = alloc_reqs.iter().map(|it| it.size.0 as i64).sum();
    Ok(TransferFromParams {
        from: *client,
        to: *VERIFIED_REGISTRY_ACTOR_ADDR,
        amount: TokenAmount::from_whole(datacap_required),
        operator_data: serialize(
            &ext::verifreg::AllocationRequests { allocations: alloc_reqs, extensions: vec![] },
            "allocation requests",
        )?,
    })
}

// Parses allocation IDs from a TransferFromReturn
fn datacap_transfer_response(ret: &RawBytes) -> Result<Vec<AllocationID>, ActorError> {
    let ret: TransferFromReturn = deserialize(ret, "transfer from response")?;
    let allocs: ext::verifreg::AllocationsResponse =
        deserialize(&ret.recipient_data, "allocations response")?;
    Ok(allocs.new_allocations)
}

pub fn gen_rand_next_epoch(
    policy: &Policy,
    start_epoch: ChainEpoch,
    deal_id: DealID,
) -> ChainEpoch {
    let offset = deal_id as i64 % policy.deal_updates_interval;
    let q = QuantSpec { unit: policy.deal_updates_interval, offset: 0 };
    let prev_day = q.quantize_down(start_epoch);
    if prev_day + offset >= start_epoch {
        return prev_day + offset;
    }
    let next_day = q.quantize_up(start_epoch);
    next_day + offset
}

////////////////////////////////////////////////////////////////////////////////
// Checks
////////////////////////////////////////////////////////////////////////////////
fn validate_deal_can_activate(
    proposal: &DealProposal,
    miner_addr: &Address,
    sector_expiration: ChainEpoch,
    curr_epoch: ChainEpoch,
) -> Result<(), ActorError> {
    if &proposal.provider != miner_addr {
        return Err(actor_error!(
            forbidden,
            "proposal has provider {}, must be {}",
            proposal.provider,
            miner_addr
        ));
    };

    if curr_epoch > proposal.start_epoch {
        return Err(actor_error!(
            illegal_argument,
            "proposal start epoch {} has already elapsed at {}",
            proposal.start_epoch,
            curr_epoch
        ));
    };

    if proposal.end_epoch > sector_expiration {
        return Err(actor_error!(
            illegal_argument,
            "proposal expiration {} exceeds sector expiration {}",
            proposal.end_epoch,
            sector_expiration
        ));
    };

    Ok(())
}

fn validate_deal(
    rt: &impl Runtime,
    deal: &ClientDealProposal,
    network_raw_power: &StoragePower,
    baseline_power: &StoragePower,
) -> Result<(), ActorError> {
    deal_proposal_is_internally_valid(rt, deal)?;

    let proposal = &deal.proposal;

    if proposal.label.len() > detail::DEAL_MAX_LABEL_SIZE {
        return Err(actor_error!(
            illegal_argument,
            "deal label can be at most {} bytes, is {}",
            detail::DEAL_MAX_LABEL_SIZE,
            proposal.label.len()
        ));
    }

    proposal
        .piece_size
        .validate()
        .map_err(|e| actor_error!(illegal_argument, "proposal piece size is invalid: {}", e))?;

    // * we are skipping the check for if Cid is defined, but this shouldn't be possible

    if !is_piece_cid(&proposal.piece_cid) {
        return Err(actor_error!(illegal_argument, "proposal PieceCID undefined"));
    }

    if proposal.end_epoch <= proposal.start_epoch {
        return Err(actor_error!(illegal_argument, "proposal end before proposal start"));
    }

    if rt.curr_epoch() > proposal.start_epoch {
        return Err(actor_error!(illegal_argument, "Deal start epoch has already elapsed."));
    };

    let (min_dur, max_dur) = deal_duration_bounds(proposal.piece_size);
    if proposal.duration() < min_dur || proposal.duration() > max_dur {
        return Err(actor_error!(illegal_argument, "Deal duration out of bounds."));
    };

    let (min_price, max_price) =
        deal_price_per_epoch_bounds(proposal.piece_size, proposal.duration());
    if proposal.storage_price_per_epoch < min_price || &proposal.storage_price_per_epoch > max_price
    {
        return Err(actor_error!(illegal_argument, "Storage price out of bounds."));
    };

    let (min_provider_collateral, max_provider_collateral) = deal_provider_collateral_bounds(
        rt.policy(),
        proposal.piece_size,
        network_raw_power,
        baseline_power,
        &rt.total_fil_circ_supply(),
    );
    if proposal.provider_collateral < min_provider_collateral
        || proposal.provider_collateral > max_provider_collateral
    {
        return Err(actor_error!(illegal_argument, "Provider collateral out of bounds."));
    };

    let (min_client_collateral, max_client_collateral) =
        deal_client_collateral_bounds(proposal.piece_size, proposal.duration());
    if proposal.client_collateral < min_client_collateral
        || proposal.client_collateral > max_client_collateral
    {
        return Err(actor_error!(illegal_argument, "Client collateral out of bounds."));
    };

    Ok(())
}

fn deal_proposal_is_internally_valid(
    rt: &impl Runtime,
    proposal: &ClientDealProposal,
) -> Result<(), ActorError> {
    let signature_bytes = proposal.client_signature.bytes.clone();
    // Generate unsigned bytes
    let proposal_bytes = serialize_vec(&proposal.proposal, "deal proposal")?;

    rt.send(
        &proposal.proposal.client,
        ext::account::AUTHENTICATE_MESSAGE_METHOD,
        RawBytes::serialize(ext::account::AuthenticateMessageParams {
            signature: signature_bytes,
            message: proposal_bytes,
        })?,
        TokenAmount::zero(),
    )
    .map_err(|e| e.wrap("proposal authentication failed"))?;
    Ok(())
}

pub const DAG_CBOR: u64 = 0x71; // TODO is there a better place to get this?

/// Compute a deal CID using the runtime.
pub(crate) fn rt_deal_cid(rt: &impl Runtime, proposal: &DealProposal) -> Result<Cid, ActorError> {
    const DIGEST_SIZE: u32 = 32;
    let data = &proposal.marshal_cbor()?;
    let hash = MultihashGeneric::wrap(Code::Blake2b256.into(), &rt.hash_blake2b(data))
        .map_err(|e| actor_error!(illegal_argument; "failed to take cid of proposal {}", e))?;
    debug_assert_eq!(u32::from(hash.size()), DIGEST_SIZE, "expected 32byte digest");
    Ok(Cid::new_v1(DAG_CBOR, hash))
}

/// Compute a deal CID directly.
pub(crate) fn deal_cid(proposal: &DealProposal) -> Result<Cid, ActorError> {
    const DIGEST_SIZE: u32 = 32;
    let data = &proposal.marshal_cbor()?;
    let hash = Code::Blake2b256.digest(data);
    debug_assert_eq!(u32::from(hash.size()), DIGEST_SIZE, "expected 32byte digest");
    Ok(Cid::new_v1(DAG_CBOR, hash))
}

fn request_miner_control_addrs(
    rt: &mut impl Runtime,
    miner_id: ActorID,
) -> Result<(Address, Address, Vec<Address>), ActorError> {
    let ret = rt.send(
        &Address::new_id(miner_id),
        ext::miner::CONTROL_ADDRESSES_METHOD,
        RawBytes::default(),
        TokenAmount::zero(),
    )?;
    let addrs: ext::miner::GetControlAddressesReturnParams = ret.deserialize()?;

    Ok((addrs.owner, addrs.worker, addrs.control_addresses))
}

/// Resolves a provider or client address to the canonical form against which a balance should be held, and
/// the designated recipient address of withdrawals (which is the same, for simple account parties).
fn escrow_address(
    rt: &mut impl Runtime,
    addr: &Address,
) -> Result<(Address, Address, Vec<Address>), ActorError> {
    // Resolve the provided address to the canonical form against which the balance is held.
    let nominal = rt
        .resolve_address(addr)
        .ok_or_else(|| actor_error!(illegal_argument, "failed to resolve address {}", addr))?;

    let code_id = rt
        .get_actor_code_cid(&nominal)
        .ok_or_else(|| actor_error!(illegal_argument, "no code for address {}", nominal))?;

    let nominal_addr = Address::new_id(nominal);

    if rt.resolve_builtin_actor_type(&code_id) == Some(Type::Miner) {
        // Storage miner actor entry; implied funds recipient is the associated owner address.
        let (owner_addr, worker_addr, _) = request_miner_control_addrs(rt, nominal)?;
        return Ok((nominal_addr, owner_addr, vec![owner_addr, worker_addr]));
    }

    Ok((nominal_addr, nominal_addr, vec![nominal_addr]))
}

/// Requests the current epoch target block reward from the reward actor.
fn request_current_baseline_power(rt: &mut impl Runtime) -> Result<StoragePower, ActorError> {
    let rwret = rt.send(
        &REWARD_ACTOR_ADDR,
        ext::reward::THIS_EPOCH_REWARD_METHOD,
        RawBytes::default(),
        TokenAmount::zero(),
    )?;
    let ret: ThisEpochRewardReturn = rwret.deserialize()?;
    Ok(ret.this_epoch_baseline_power)
}

/// Requests the current network total power and pledge from the power actor.
/// Returns a tuple of (raw_power, qa_power).
fn request_current_network_power(
    rt: &mut impl Runtime,
) -> Result<(StoragePower, StoragePower), ActorError> {
    let rwret = rt.send(
        &STORAGE_POWER_ACTOR_ADDR,
        ext::power::CURRENT_TOTAL_POWER_METHOD,
        RawBytes::default(),
        TokenAmount::zero(),
    )?;
    let ret: ext::power::CurrentTotalPowerReturnParams = rwret.deserialize()?;
    Ok((ret.raw_byte_power, ret.quality_adj_power))
}

pub fn deal_id_key(k: DealID) -> BytesKey {
    let bz = k.encode_var_vec();
    bz.into()
}

impl ActorCode for Actor {
    fn invoke_method<RT>(
        rt: &mut RT,
        method: MethodNum,
        params: &RawBytes,
    ) -> Result<RawBytes, ActorError>
    where
        RT: Runtime,
    {
        restrict_internal_api(rt, method)?;
        match FromPrimitive::from_u64(method) {
            Some(Method::Constructor) => {
                Self::constructor(rt)?;
                Ok(RawBytes::default())
            }
            Some(Method::AddBalance) | Some(Method::AddBalanceExported) => {
                Self::add_balance(rt, cbor::deserialize_params(params)?)?;
                Ok(RawBytes::default())
            }
            Some(Method::WithdrawBalance) | Some(Method::WithdrawBalanceExported) => {
                let res = Self::withdraw_balance(rt, cbor::deserialize_params(params)?)?;
                Ok(RawBytes::serialize(res)?)
            }
            Some(Method::PublishStorageDeals) | Some(Method::PublishStorageDealsExported) => {
                let res = Self::publish_storage_deals(rt, cbor::deserialize_params(params)?)?;
                Ok(RawBytes::serialize(res)?)
            }
            Some(Method::VerifyDealsForActivation) => {
                let res = Self::verify_deals_for_activation(rt, cbor::deserialize_params(params)?)?;
                Ok(RawBytes::serialize(res)?)
            }
            Some(Method::ActivateDeals) => {
                let res = Self::activate_deals(rt, cbor::deserialize_params(params)?)?;
                Ok(RawBytes::serialize(res)?)
            }
            Some(Method::OnMinerSectorsTerminate) => {
                Self::on_miner_sectors_terminate(rt, cbor::deserialize_params(params)?)?;
                Ok(RawBytes::default())
            }
            Some(Method::ComputeDataCommitment) => {
                let res = Self::compute_data_commitment(rt, cbor::deserialize_params(params)?)?;
                Ok(RawBytes::serialize(res)?)
            }
            Some(Method::CronTick) => {
                Self::cron_tick(rt)?;
                Ok(RawBytes::default())
            }
            Some(Method::GetBalanceExported) => {
                let res = Self::get_balance(rt, cbor::deserialize_params(params)?)?;
                Ok(RawBytes::serialize(res)?)
            }
            Some(Method::GetDealDataCommitmentExported) => {
                let res = Self::get_deal_data_commitment(rt, cbor::deserialize_params(params)?)?;
                Ok(RawBytes::serialize(res)?)
            }
            Some(Method::GetDealClientExported) => {
                let res = Self::get_deal_client(rt, cbor::deserialize_params(params)?)?;
                Ok(RawBytes::serialize(res)?)
            }
            Some(Method::GetDealProviderExported) => {
                let res = Self::get_deal_provider(rt, cbor::deserialize_params(params)?)?;
                Ok(RawBytes::serialize(res)?)
            }
            Some(Method::GetDealLabelExported) => {
                let res = Self::get_deal_label(rt, cbor::deserialize_params(params)?)?;
                Ok(RawBytes::serialize(res)?)
            }
            Some(Method::GetDealTermExported) => {
                let res = Self::get_deal_term(rt, cbor::deserialize_params(params)?)?;
                Ok(RawBytes::serialize(res)?)
            }
            Some(Method::GetDealTotalPriceExported) => {
                let res = Self::get_deal_total_price(rt, cbor::deserialize_params(params)?)?;
                Ok(RawBytes::serialize(res)?)
            }
            Some(Method::GetDealClientCollateralExported) => {
                let res = Self::get_deal_client_collateral(rt, cbor::deserialize_params(params)?)?;
                Ok(RawBytes::serialize(res)?)
            }
            Some(Method::GetDealProviderCollateralExported) => {
                let res =
                    Self::get_deal_provider_collateral(rt, cbor::deserialize_params(params)?)?;
                Ok(RawBytes::serialize(res)?)
            }
            Some(Method::GetDealVerifiedExported) => {
                let res = Self::get_deal_verified(rt, cbor::deserialize_params(params)?)?;
                Ok(RawBytes::serialize(res)?)
            }
            Some(Method::GetDealActivationExported) => {
                let res = Self::get_deal_activation(rt, cbor::deserialize_params(params)?)?;
                Ok(RawBytes::serialize(res)?)
            }
            None => Err(actor_error!(unhandled_message, "Invalid method")),
        }
    }
}
