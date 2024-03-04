// Copyright 2019-2022 ChainSafe Systems
// SPDX-License-Identifier: Apache-2.0, MIT

use std::cmp::min;
use std::collections::{BTreeMap, BTreeSet, HashSet};

use cid::multihash::{Code, MultihashGeneric};
use cid::Cid;
use fil_actors_runtime::reward::ThisEpochRewardReturn;
use frc46_token::token::types::{BalanceReturn, TransferFromParams, TransferFromReturn};
use fvm_ipld_bitfield::BitField;
use fvm_ipld_blockstore::Blockstore;
use fvm_ipld_encoding::ipld_block::IpldBlock;
use fvm_ipld_encoding::{RawBytes, DAG_CBOR};
use fvm_ipld_hamt::BytesKey;
use fvm_shared::address::Address;
use fvm_shared::bigint::BigInt;
use fvm_shared::clock::{ChainEpoch, EPOCH_UNDEFINED};
use fvm_shared::deal::DealID;
use fvm_shared::econ::TokenAmount;
use fvm_shared::error::ExitCode;
use fvm_shared::piece::PieceInfo;
use fvm_shared::sector::{RegisteredSealProof, SectorNumber, SectorSize, StoragePower};
use fvm_shared::sys::SendFlags;
use fvm_shared::{ActorID, METHOD_CONSTRUCTOR, METHOD_SEND};
use integer_encoding::VarInt;
use log::{info, warn};
use num_derive::FromPrimitive;
use num_traits::Zero;

use fil_actors_runtime::cbor::{deserialize, serialize};
use fil_actors_runtime::runtime::builtins::Type;
use fil_actors_runtime::runtime::{ActorCode, Policy, Runtime};
use fil_actors_runtime::{
    actor_dispatch, actor_error, deserialize_block, ActorContext, ActorDowncast, ActorError,
    AsActorError, BURNT_FUNDS_ACTOR_ADDR, CRON_ACTOR_ADDR, DATACAP_TOKEN_ACTOR_ADDR,
    REWARD_ACTOR_ADDR, STORAGE_POWER_ACTOR_ADDR, SYSTEM_ACTOR_ADDR, VERIFIED_REGISTRY_ACTOR_ADDR,
};
use fil_actors_runtime::{extract_send_result, BatchReturnGen, FIRST_ACTOR_SPECIFIC_EXIT_CODE};

use crate::balance_table::BalanceTable;
use crate::ext::verifreg::{AllocationID, AllocationRequest};

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
mod emit;
mod state;
mod types;

#[cfg(feature = "fil-actor")]
fil_actors_runtime::wasm_trampoline!(Actor);

pub const NO_ALLOCATION_ID: u64 = 0;

// Indicates that information about a past deal is no longer available.
pub const EX_DEAL_EXPIRED: ExitCode = ExitCode::new(FIRST_ACTOR_SPECIFIC_EXIT_CODE);
// Indicates that information about a deal's activation is not yet available.
pub const EX_DEAL_NOT_ACTIVATED: ExitCode = ExitCode::new(FIRST_ACTOR_SPECIFIC_EXIT_CODE + 1);

/// Market actor methods available
#[derive(FromPrimitive)]
#[repr(u64)]
pub enum Method {
    Constructor = METHOD_CONSTRUCTOR,
    AddBalance = 2,
    WithdrawBalance = 3,
    PublishStorageDeals = 4,
    VerifyDealsForActivation = 5,
    BatchActivateDeals = 6,
    OnMinerSectorsTerminate = 7,
    // ComputeDataCommitment = 8, // Deprecated
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
    GetDealSectorExported = frc42_dispatch::method_hash!("GetDealSector"),
    SettleDealPaymentsExported = frc42_dispatch::method_hash!("SettleDealPayments"),
    SectorContentChangedExported = ext::miner::SECTOR_CONTENT_CHANGED,
}

/// Market Actor
pub struct Actor;

impl Actor {
    pub fn constructor(rt: &impl Runtime) -> Result<(), ActorError> {
        rt.validate_immediate_caller_is(std::iter::once(&SYSTEM_ACTOR_ADDR))?;

        let st = State::new(rt.store())?;
        rt.create(&st)?;
        Ok(())
    }

    /// Deposits the received value into the balance held in escrow.
    fn add_balance(rt: &impl Runtime, params: AddBalanceParams) -> Result<(), ActorError> {
        let msg_value = rt.message().value_received();

        if msg_value <= TokenAmount::zero() {
            return Err(actor_error!(
                illegal_argument,
                "balance to add must be greater than zero was: {}",
                msg_value
            ));
        }

        rt.validate_immediate_caller_accept_any()?;

        let (nominal, _, _) = escrow_address(rt, &params.provider_or_client)?;

        rt.transaction(|st: &mut State, rt| {
            st.add_balance_to_escrow_table(rt.store(), &nominal, &msg_value)?;
            Ok(())
        })?;

        Ok(())
    }

    /// Attempt to withdraw the specified amount from the balance held in escrow.
    /// If less than the specified amount is available, yields the entire available balance.
    fn withdraw_balance(
        rt: &impl Runtime,
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

        extract_send_result(rt.send_simple(
            &recipient,
            METHOD_SEND,
            None,
            amount_extracted.clone(),
        ))?;

        Ok(WithdrawBalanceReturn { amount_withdrawn: amount_extracted })
    }

    /// Returns the escrow balance and locked amount for an address.
    fn get_balance(
        rt: &impl Runtime,
        params: GetBalanceParams,
    ) -> Result<GetBalanceReturn, ActorError> {
        rt.validate_immediate_caller_accept_any()?;
        let account = params.account;
        let nominal = rt.resolve_address(&account).ok_or_else(|| {
            actor_error!(illegal_argument, "failed to resolve address {}", account)
        })?;
        let account = Address::new_id(nominal);

        let store = rt.store();
        let st: State = rt.state()?;
        let balances = BalanceTable::from_root(store, &st.escrow_table, "escrow table")?;
        let locks = BalanceTable::from_root(store, &st.locked_table, "locked table")?;
        let balance = balances.get(&account)?;
        let locked = locks.get(&account)?;

        Ok(GetBalanceReturn { balance, locked })
    }

    /// Publish a new set of storage deals (not yet included in a sector).
    fn publish_storage_deals(
        rt: &impl Runtime,
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

        let caller = rt.message().caller();
        let caller_status: ext::miner::IsControllingAddressReturn =
            deserialize_block(extract_send_result(rt.send_simple(
                &Address::new_id(provider_id),
                ext::miner::IS_CONTROLLING_ADDRESS_EXPORTED,
                IpldBlock::serialize_cbor(&ext::miner::IsControllingAddressParam {
                    address: caller,
                })?,
                TokenAmount::zero(),
            ))?)?;
        if !caller_status.is_controlling {
            return Err(actor_error!(
                forbidden,
                "caller {} is not worker or control address of provider {}",
                caller,
                provider_id
            ));
        }
        // Deals that passed `AuthenticateMessage` and other state-less checks.
        let mut validity_index: Vec<bool> = Vec::with_capacity(params.deals.len());

        let baseline_power = request_current_baseline_power(rt)?;
        let (network_raw_power, _) = request_current_network_power(rt)?;

        // We perform these checks before loading state since the call to `AuthenticateMessage` could recurse
        for (di, deal) in params.deals.iter().enumerate() {
            let valid = if let Err(e) = validate_deal(rt, deal, &network_raw_power, &baseline_power)
            {
                info!("invalid deal {}: {}", di, e);
                false
            } else {
                true
            };

            validity_index.push(valid);
        }

        struct ValidDeal {
            proposal: DealProposal,
            serialized_proposal: RawBytes,
            cid: Cid,
        }

        // Deals that passed validation.
        let mut valid_deals: Vec<ValidDeal> = Vec::with_capacity(params.deals.len());
        // CIDs of valid proposals.
        let mut proposal_cid_lookup = BTreeSet::new();
        let mut total_client_lockup: BTreeMap<ActorID, TokenAmount> = BTreeMap::new();
        // Client datacap balance remaining after allocations for deals processed so far.
        let mut client_datacap_remaining: BTreeMap<ActorID, TokenAmount> = BTreeMap::new();
        // Verified allocation requests to make for each client, paired with the proposal CID.
        let mut client_alloc_reqs: BTreeMap<ActorID, Vec<(Cid, AllocationRequest)>> =
            BTreeMap::new();
        let mut total_provider_lockup = TokenAmount::zero();

        let mut valid_input_bf = BitField::default();
        let curr_epoch = rt.curr_epoch();

        let state: State = rt.state()?;

        for (di, mut deal) in params.deals.into_iter().enumerate() {
            if !*validity_index.get(di).context_code(
                ExitCode::USR_ASSERTION_FAILED,
                "validity index has incorrect length",
            )? {
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

            let serialized_proposal = serialize(&deal.proposal, "normalized deal proposal")
                .context_code(ExitCode::USR_SERIALIZATION, "failed to serialize")?;
            let pcid = serialized_deal_cid(rt, &serialized_proposal).map_err(
                |e| actor_error!(illegal_argument; "failed to take cid of proposal {}: {}", di, e),
            )?;

            // check proposalCids for duplication within message batch
            // check state PendingProposals for duplication across messages
            let duplicate_in_state = state.has_pending_deal(rt.store(), &pcid)?;

            let duplicate_in_message = proposal_cid_lookup.contains(&pcid);
            if duplicate_in_state || duplicate_in_message {
                info!("invalid deal {}: cannot publish duplicate deal proposal", di);
                continue;
            }

            // Fetch each client's datacap balance and calculate the amount of datacap required for
            // each client's verified deals.
            // Drop any verified deals for which the client has insufficient datacap.
            if deal.proposal.verified_deal {
                let remaining_datacap = match client_datacap_remaining.get(&client_id).cloned() {
                    None => balance_of(rt, &Address::new_id(client_id))
                        .with_context_code(ExitCode::USR_NOT_FOUND, || {
                            format!("failed to get datacap balance for client {}", client_id)
                        })?,
                    Some(client_data) => client_data,
                };
                let piece_datacap_required =
                    TokenAmount::from_whole(deal.proposal.piece_size.0 as i64);
                if remaining_datacap < piece_datacap_required {
                    client_datacap_remaining.insert(client_id, remaining_datacap);
                    continue; // Drop the deal
                }
                client_datacap_remaining
                    .insert(client_id, remaining_datacap - piece_datacap_required);
                client_alloc_reqs
                    .entry(client_id)
                    .or_default()
                    .push((pcid, alloc_request_for_deal(&deal.proposal, rt.policy(), curr_epoch)));
            }

            total_provider_lockup = provider_lockup;
            total_client_lockup.insert(client_id, client_lockup);
            proposal_cid_lookup.insert(pcid);
            valid_deals.push(ValidDeal { proposal: deal.proposal, serialized_proposal, cid: pcid });
            valid_input_bf.set(di as u64)
        }

        // Make datacap allocation requests by transferring datacap tokens, once per client.
        // Record the allocation ID for each deal proposal CID.
        let mut deal_allocation_ids: BTreeMap<Cid, AllocationID> = BTreeMap::new();
        for (client_id, cids_and_reqs) in client_alloc_reqs.iter() {
            let reqs: Vec<AllocationRequest> =
                cids_and_reqs.iter().map(|(_, req)| req.clone()).collect();
            let params = datacap_transfer_request(&Address::new_id(*client_id), reqs)?;
            // A datacap transfer is all-or-nothing.
            // We expect it to succeed because we checked the client's balance earlier.
            let alloc_ids = transfer_from(rt, params)
                .with_context_code(ExitCode::USR_ILLEGAL_STATE, || {
                    format!("failed to transfer datacap from client {}", *client_id)
                })?;
            if alloc_ids.len() != cids_and_reqs.len() {
                return Err(
                    actor_error!(illegal_state; "datacap transfer returned {} allocation IDs for {} requests",
                        alloc_ids.len(), cids_and_reqs.len()),
                );
            }
            for ((cid, _), alloc_id) in cids_and_reqs.iter().zip(alloc_ids.iter()) {
                deal_allocation_ids.insert(*cid, *alloc_id);
            }
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
            let mut pending_deal_allocation_ids: Vec<(DealID, AllocationID)> = vec![];

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
                if let Some(alloc_id) = deal_allocation_ids.get(&valid_deal.cid) {
                    pending_deal_allocation_ids.push((deal_id, *alloc_id));
                }

                // Randomize the first epoch for when the deal will be processed so an attacker isn't able to
                // schedule too many deals for the same tick.
                deals_by_epoch.push((
                    next_update_epoch(
                        deal_id,
                        rt.policy().deal_updates_interval,
                        valid_deal.proposal.start_epoch,
                    ),
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

        // notify clients, any failures cause the entire publish_storage_deals method to fail
        // it's unsafe to ignore errors here, since that could be used to attack storage contract clients
        // that might be unaware they're making storage deals
        for (valid_deal, &deal_id) in valid_deals.iter().zip(&new_deal_ids) {
            _ = extract_send_result(rt.send_simple(
                &valid_deal.proposal.client,
                MARKET_NOTIFY_DEAL_METHOD,
                IpldBlock::serialize_cbor(&MarketNotifyDealParams {
                    proposal: valid_deal.serialized_proposal.to_vec(),
                    deal_id,
                })?,
                TokenAmount::zero(),
            ))
            .with_context_code(ExitCode::USR_ILLEGAL_ARGUMENT, || {
                format!("failed to notify deal with proposal cid {}", valid_deal.cid)
            })?;

            emit::deal_published(
                rt,
                valid_deal.proposal.client.id().unwrap(),
                valid_deal.proposal.provider.id().unwrap(),
                deal_id,
            )?;
        }

        Ok(PublishStorageDealsReturn { ids: new_deal_ids, valid_deals: valid_input_bf })
    }

    /// Verify that a given set of storage deals is valid for a sector currently being PreCommitted
    /// and return UnsealedCID for the set of deals.
    fn verify_deals_for_activation(
        rt: &impl Runtime,
        params: VerifyDealsForActivationParams,
    ) -> Result<VerifyDealsForActivationReturn, ActorError> {
        rt.validate_immediate_caller_type(std::iter::once(&Type::Miner))?;
        let miner_addr = rt.message().caller();
        let curr_epoch = rt.curr_epoch();

        let st: State = rt.state()?;
        let proposal_array = st.load_proposals(rt.store())?;

        let mut unsealed_cids = Vec::with_capacity(params.sectors.len());
        for sector in params.sectors.iter() {
            let sector_proposals = get_proposals(&proposal_array, &sector.deal_ids, st.next_id)?;
            let sector_size = sector
                .sector_type
                .sector_size()
                .map_err(|e| actor_error!(illegal_argument, "sector size unknown: {}", e))?;
            validate_deals_for_sector(
                &sector_proposals,
                &miner_addr,
                sector.sector_expiry,
                curr_epoch,
                Some(sector_size),
            )
            .context("failed to validate deal proposals for activation")?;

            let commd = if sector.deal_ids.is_empty() {
                None
            } else {
                let proposals_iter = sector_proposals.iter().map(|(_, p)| p);
                Some(compute_data_commitment(rt, proposals_iter, sector.sector_type)?)
            };

            unsealed_cids.push(commd);
        }

        Ok(VerifyDealsForActivationReturn { unsealed_cids })
    }

    /// Activate a set of deals grouped by sector, returning the size and
    /// extra info about verified deals.
    /// Sectors' deals are activated in parameter-defined order.
    /// Each sector's deals are activated or fail as a group, but independently of other sectors.
    /// Note that confirming all deals fit within a sector is the caller's responsibility
    /// (and is implied by confirming the sector's data commitment is derived from the deal peices).
    // see https://github.com/filecoin-project/builtin-actors/issues/1308
    fn batch_activate_deals(
        rt: &impl Runtime,
        params: BatchActivateDealsParams,
    ) -> Result<BatchActivateDealsResult, ActorError> {
        rt.validate_immediate_caller_type(std::iter::once(&Type::Miner))?;
        let miner_addr = rt.message().caller();
        let curr_epoch = rt.curr_epoch();

        let (activations, batch_ret) = rt.transaction(|st: &mut State, rt| {
            let proposals = st.load_proposals(rt.store())?;
            let states = st.load_deal_states(rt.store())?;
            let pending_deals = st.load_pending_deals(rt.store())?;
            let mut pending_deal_allocation_ids =
                st.load_pending_deal_allocation_ids(rt.store())?;

            let mut deal_states: Vec<(DealID, DealState)> = vec![];
            let mut batch_gen = BatchReturnGen::new(params.sectors.len());
            let mut activations: Vec<SectorDealActivation> = vec![];
            let mut activated_deals: HashSet<DealID> = HashSet::new();
            let mut sectors_deals: Vec<(SectorNumber, Vec<DealID>)> = vec![];

            'sector: for sector in params.sectors {
                let mut sector_deal_ids = sector.deal_ids.clone();
                sector_deal_ids.sort();
                if sector_deal_ids.windows(2).any(|w| w[0] == w[1]) {
                    log::warn!("failed to activate sector, duplicate deal");
                    batch_gen.add_fail(ExitCode::USR_ILLEGAL_ARGUMENT);
                    continue;
                }
                let mut validated_proposals = vec![];
                // Iterate once to validate all the requested deals.
                // If a deal fails, skip the whole sector.
                for &deal_id in &sector.deal_ids {
                    // Check each deal is present only once, within and across sectors.
                    if activated_deals.contains(&deal_id) {
                        log::warn!("failed to activate sector, duplicated deal {}", deal_id);
                        batch_gen.add_fail(ExitCode::USR_ILLEGAL_ARGUMENT);
                        continue 'sector;
                    }

                    let proposal = match preactivate_deal(
                        rt,
                        deal_id,
                        &proposals,
                        &states,
                        &pending_deals,
                        &miner_addr,
                        sector.sector_expiry,
                        curr_epoch,
                        st.next_id,
                    )? {
                        Ok(v) => v,
                        Err(e) => {
                            log::warn!("failed to activate deal: {}", e);
                            batch_gen.add_fail(e.exit_code());
                            continue 'sector;
                        }
                    };
                    validated_proposals.push(proposal);
                }

                let mut activated = vec![];
                // Given that all deals validated, prepare the state updates for them all.
                // There's no continue below here to ensure updates are consistent.
                // Any error must abort.
                for (deal_id, proposal) in sector.deal_ids.iter().zip(&validated_proposals) {
                    activated_deals.insert(*deal_id);
                    // Extract and remove any verified allocation ID for the pending deal.
                    let alloc_id =
                        pending_deal_allocation_ids.delete(deal_id)?.unwrap_or(NO_ALLOCATION_ID);

                    activated.push(ActivatedDeal {
                        client: proposal.client.id().unwrap(),
                        allocation_id: alloc_id,
                        data: proposal.piece_cid,
                        size: proposal.piece_size,
                    });

                    // Prepare initial deal state.
                    deal_states.push((
                        *deal_id,
                        DealState {
                            sector_number: sector.sector_number,
                            sector_start_epoch: curr_epoch,
                            last_updated_epoch: EPOCH_UNDEFINED,
                            slash_epoch: EPOCH_UNDEFINED,
                        },
                    ));
                }

                let data_commitment = if params.compute_cid && !sector.deal_ids.is_empty() {
                    Some(compute_data_commitment(rt, &validated_proposals, sector.sector_type)?)
                } else {
                    None
                };

                sectors_deals.push((sector.sector_number, sector.deal_ids.clone()));
                activations.push(SectorDealActivation { activated, unsealed_cid: data_commitment });

                for (deal_id, proposal) in sector.deal_ids.iter().zip(&validated_proposals) {
                    emit::deal_activated(
                        rt,
                        *deal_id,
                        proposal.client.id().unwrap(),
                        proposal.provider.id().unwrap(),
                    )?;
                }

                batch_gen.add_success();
            }

            st.put_deal_states(rt.store(), &deal_states)?;
            st.put_sector_deal_ids(rt.store(), miner_addr.id().unwrap(), &sectors_deals)?;
            st.save_pending_deal_allocation_ids(&mut pending_deal_allocation_ids)?;
            Ok((activations, batch_gen.gen()))
        })?;

        Ok(BatchActivateDealsResult { activations, activation_results: batch_ret })
    }

    /// Receives notification of a change to sector content, which may satisfy to activate a deal.
    /// Deals are activated or fail independently, including in the same sector.
    /// This is an alternative to ActivateDeals.
    fn sector_content_changed(
        rt: &impl Runtime,
        params: ext::miner::SectorContentChangedParams,
    ) -> Result<ext::miner::SectorContentChangedReturn, ActorError> {
        rt.validate_immediate_caller_type(std::iter::once(&Type::Miner))?;
        let miner_addr = rt.message().caller();
        let curr_epoch = rt.curr_epoch();

        let sectors_ret = rt.transaction(|st: &mut State, rt| {
            let proposals = st.load_proposals(rt.store())?;
            let states = st.load_deal_states(rt.store())?;
            let pending_deals = st.load_pending_deals(rt.store())?;
            let mut pending_deal_allocation_ids =
                st.load_pending_deal_allocation_ids(rt.store())?;

            let mut deal_states: Vec<(DealID, DealState)> = vec![];
            let mut activated_deals: HashSet<DealID> = HashSet::new();
            let mut sectors_deals: Vec<(SectorNumber, Vec<DealID>)> = vec![];
            let mut sectors_ret: Vec<ext::miner::SectorReturn> = vec![];

            for sector in &params.sectors {
                let mut sector_deal_ids: Vec<DealID> = vec![];
                let mut pieces_ret: Vec<_> =
                    vec![ext::miner::PieceReturn { accepted: false }; sector.added.len()];
                for (piece, ret) in sector.added.iter().zip(&mut pieces_ret) {
                    let deal_id: DealID = match deserialize(&piece.payload, "deal id") {
                        Ok(v) => v,
                        Err(e) => {
                            log::warn!("failed to deserialize deal id {:?}: {}", piece.payload, e);
                            continue;
                        }
                    };
                    if activated_deals.contains(&deal_id) {
                        log::warn!("duplicated deal {}", deal_id);
                        continue;
                    }

                    let proposal = match preactivate_deal(
                        rt,
                        deal_id,
                        &proposals,
                        &states,
                        &pending_deals,
                        &miner_addr,
                        sector.minimum_commitment_epoch,
                        curr_epoch,
                        st.next_id,
                    )? {
                        Ok(id) => id,
                        Err(e) => {
                            log::warn!("failed to activate deal {}: {}", deal_id, e);
                            continue;
                        }
                    };

                    if piece.data != proposal.piece_cid {
                        log::warn!(
                            "deal {} piece CID {} doesn't match {}",
                            deal_id,
                            piece.data,
                            proposal.piece_cid
                        );
                        continue;
                    }
                    if piece.size != proposal.piece_size {
                        log::warn!(
                            "deal {} piece size {} doesn't match {}",
                            deal_id,
                            piece.size.0,
                            proposal.piece_size.0
                        );
                        continue;
                    }

                    // No continue below here, to ensure state changes are consistent.
                    activated_deals.insert(deal_id);

                    emit::deal_activated(
                        rt,
                        deal_id,
                        proposal.client.id().unwrap(),
                        proposal.provider.id().unwrap(),
                    )?;

                    // Remove any verified allocation ID for the pending deal.
                    pending_deal_allocation_ids.delete(&deal_id)?;

                    deal_states.push((
                        deal_id,
                        DealState {
                            sector_number: sector.sector,
                            sector_start_epoch: curr_epoch,
                            last_updated_epoch: EPOCH_UNDEFINED,
                            slash_epoch: EPOCH_UNDEFINED,
                        },
                    ));
                    sector_deal_ids.push(deal_id);
                    ret.accepted = true;
                }

                sectors_deals.push((sector.sector, sector_deal_ids));
                assert_eq!(pieces_ret.len(), sector.added.len(), "mismatched piece returns");
                sectors_ret.push(ext::miner::SectorReturn { added: pieces_ret });
            }
            st.put_deal_states(rt.store(), &deal_states)?;
            st.put_sector_deal_ids(rt.store(), miner_addr.id().unwrap(), &sectors_deals)?;
            st.save_pending_deal_allocation_ids(&mut pending_deal_allocation_ids)?;

            assert_eq!(sectors_ret.len(), params.sectors.len(), "mismatched sector returns");
            Ok(sectors_ret)
        })?;

        Ok(ext::miner::SectorContentChangedReturn { sectors: sectors_ret })
    }

    /// Terminate a set of deals in response to their containing sector being terminated.
    /// Slash provider collateral, refund client collateral, and refund partial unpaid escrow
    /// amount to client.
    fn on_miner_sectors_terminate(
        rt: &impl Runtime,
        params: OnMinerSectorsTerminateParams,
    ) -> Result<(), ActorError> {
        rt.validate_immediate_caller_type(std::iter::once(&Type::Miner))?;
        let miner_addr = rt.message().caller();

        let burn_amount = rt.transaction(|st: &mut State, rt| {
            // The sector deals mapping is removed all at once.
            // Note there may be some deal states that are not removed here,
            // despite deletion of this mapping, e.g. for expired but not-yet-settled deals.
            // The sector->deal mapping is no longer needed (the deal state has sector number too).
            let all_deal_ids = st.pop_sector_deal_ids(
                rt.store(),
                miner_addr.id().unwrap(),
                params.sectors.iter(),
            )?;

            let mut total_slashed = TokenAmount::zero();
            for id in all_deal_ids {
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

                // If a deal is already slashed, there should be no existing state for it
                // but we process it here for deletion anyway
                if state.slash_epoch != EPOCH_UNDEFINED {
                    warn!("deal {}, already slashed, terminating now anyway", id);
                }

                // Deals that were never processed may still have a pending proposal linked
                if state.last_updated_epoch == EPOCH_UNDEFINED {
                    let dcid = deal_cid(rt, &deal)?;
                    st.remove_pending_deal(rt.store(), dcid)?;
                }

                state.slash_epoch = params.epoch;
                total_slashed += st.process_slashed_deal(rt.store(), &deal, &state)?;
                st.remove_completed_deal(rt.store(), id)?;

                emit::deal_terminated(
                    rt,
                    id,
                    deal.client.id().unwrap(),
                    deal.provider.id().unwrap(),
                )?;
            }

            Ok(total_slashed)
        })?;

        if burn_amount.is_positive() {
            extract_send_result(rt.send_simple(
                &BURNT_FUNDS_ACTOR_ADDR,
                METHOD_SEND,
                None,
                burn_amount,
            ))?;
        }
        Ok(())
    }

    fn cron_tick(rt: &impl Runtime) -> Result<(), ActorError> {
        rt.validate_immediate_caller_is(std::iter::once(&CRON_ACTOR_ADDR))?;

        let mut amount_slashed = TokenAmount::zero();
        let curr_epoch = rt.curr_epoch();

        rt.transaction(|st: &mut State, rt| {
            let last_cron = st.last_cron;
            let mut provider_deals_to_remove =
                BTreeMap::<ActorID, BTreeMap<SectorNumber, Vec<DealID>>>::new();
            let mut new_updates_scheduled: BTreeMap<ChainEpoch, Vec<DealID>> = BTreeMap::new();
            let mut epochs_completed: Vec<ChainEpoch> = vec![];

            for i in (last_cron + 1)..=rt.curr_epoch() {
                let deal_ids = st.get_deals_for_epoch(rt.store(), i)?;

                for deal_id in deal_ids {
                    let deal_proposal = match st.find_proposal(rt.store(), deal_id)? {
                        Some(dp) => dp,
                        // proposal might have been cleaned up by manual settlement or termination prior to reaching
                        // this scheduled cron tick. nothing more to do for this deal
                        None => continue,
                    };

                    let dcid = deal_cid(rt, &deal_proposal)?;

                    let mut state = match st.get_active_deal_or_process_timeout(
                        rt.store(),
                        curr_epoch,
                        deal_id,
                        &deal_proposal,
                        &dcid,
                    )? {
                        LoadDealState::Loaded(state) => state,
                        LoadDealState::ProposalExpired(expiration_penalty) => {
                            amount_slashed += expiration_penalty;
                            continue;
                        }
                        LoadDealState::TooEarly => {
                            return Err(actor_error!(
                                illegal_state,
                                "deal {} processed before start epoch {}",
                                deal_id,
                                deal_proposal.start_epoch
                            ))
                        }
                    };

                    if state.last_updated_epoch == EPOCH_UNDEFINED {
                        st.remove_pending_deal(rt.store(), dcid)?.ok_or_else(|| {
                            actor_error!(
                                illegal_state,
                                "failed to delete pending proposal: does not exist"
                            )
                        })?;

                        // newly activated deals are not scheduled for cron processing. they are handled explicitly by
                        // calling ProcessDealUpdates method with specific deal ids.
                        // the code below this point handles legacy deals that are already scheduled for cron processing
                        continue;
                    }

                    // https://github.com/filecoin-project/builtin-actors/issues/1389
                    // handling of legacy deals is still done in cron. we handle such deals here and continue to
                    // reschedule them. eventually, all legacy deals will expire and the below code can be removed.
                    let (slash_amount, _payment_amount, completed, remove_deal) = st
                        .process_deal_update(
                            rt.store(),
                            &state,
                            &deal_proposal,
                            &dcid,
                            curr_epoch,
                        )?;

                    if remove_deal {
                        // TODO: remove handling for terminated-deal slashing when marked-for-termination deals are all processed
                        // https://github.com/filecoin-project/builtin-actors/issues/1388
                        amount_slashed += slash_amount;

                        // Delete proposal and state simultaneously.
                        st.remove_completed_deal(rt.store(), deal_id)?;
                        // All proposals are stored with normalised addresses.
                        let provider = deal_proposal.provider.id().unwrap();
                        provider_deals_to_remove
                            .entry(provider)
                            .or_default()
                            .entry(state.sector_number)
                            .or_default()
                            .push(deal_id);

                        if !completed {
                            emit::deal_terminated(
                                rt,
                                deal_id,
                                deal_proposal.client.id().unwrap(),
                                deal_proposal.provider.id().unwrap(),
                            )?;
                        }
                    } else {
                        if !slash_amount.is_zero() {
                            return Err(actor_error!(
                                illegal_state,
                                "continuing deal {} should not be slashed",
                                deal_id
                            ));
                        }

                        state.last_updated_epoch = curr_epoch;
                        st.put_deal_states(rt.store(), &[(deal_id, state)])?;

                        // Compute and record the next epoch in which this deal will be updated.
                        // This epoch is independent of the deal's stated start and end epochs
                        // in order to prevent intentional scheduling of many deals for the same
                        // update epoch.
                        let next_epoch = next_update_epoch(
                            deal_id,
                            rt.policy().deal_updates_interval,
                            curr_epoch + 1,
                        );
                        new_updates_scheduled.entry(next_epoch).or_default().push(deal_id);
                    }

                    if completed {
                        emit::deal_completed(
                            rt,
                            deal_id,
                            deal_proposal.client.id().unwrap(),
                            deal_proposal.provider.id().unwrap(),
                        )?;
                    }
                }
                epochs_completed.push(i);
            }
            // Remove the provider->sector->deal mappings.
            // The sectors may still have other deals, so we can't remove the sector altogether.
            st.remove_sector_deal_ids(rt.store(), &provider_deals_to_remove)?;
            st.remove_deals_by_epoch(rt.store(), &epochs_completed)?;
            st.put_batch_deals_by_epoch(rt.store(), &new_updates_scheduled)?;
            st.last_cron = rt.curr_epoch();
            Ok(())
        })?;

        if !amount_slashed.is_zero() {
            extract_send_result(rt.send_simple(
                &BURNT_FUNDS_ACTOR_ADDR,
                METHOD_SEND,
                None,
                amount_slashed,
            ))?;
        }
        Ok(())
    }

    /// Returns the data commitment and size of a deal proposal.
    /// This will be available after the deal is published (whether or not is is activated)
    /// and up until some undefined period after it is terminated.
    fn get_deal_data_commitment(
        rt: &impl Runtime,
        params: GetDealDataCommitmentParams,
    ) -> Result<GetDealDataCommitmentReturn, ActorError> {
        rt.validate_immediate_caller_accept_any()?;
        let found = rt.state::<State>()?.get_proposal(rt.store(), params.id)?;
        Ok(GetDealDataCommitmentReturn { data: found.piece_cid, size: found.piece_size })
    }

    /// Returns the client of a deal proposal.
    fn get_deal_client(
        rt: &impl Runtime,
        params: GetDealClientParams,
    ) -> Result<GetDealClientReturn, ActorError> {
        rt.validate_immediate_caller_accept_any()?;
        let found = rt.state::<State>()?.get_proposal(rt.store(), params.id)?;
        Ok(GetDealClientReturn { client: found.client.id().unwrap() })
    }

    /// Returns the provider of a deal proposal.
    fn get_deal_provider(
        rt: &impl Runtime,
        params: GetDealProviderParams,
    ) -> Result<GetDealProviderReturn, ActorError> {
        rt.validate_immediate_caller_accept_any()?;
        let found = rt.state::<State>()?.get_proposal(rt.store(), params.id)?;
        Ok(GetDealProviderReturn { provider: found.provider.id().unwrap() })
    }

    /// Returns the label of a deal proposal.
    fn get_deal_label(
        rt: &impl Runtime,
        params: GetDealLabelParams,
    ) -> Result<GetDealLabelReturn, ActorError> {
        rt.validate_immediate_caller_accept_any()?;
        let found = rt.state::<State>()?.get_proposal(rt.store(), params.id)?;
        Ok(GetDealLabelReturn { label: found.label })
    }

    /// Returns the start epoch and duration (in epochs) of a deal proposal.
    fn get_deal_term(
        rt: &impl Runtime,
        params: GetDealTermParams,
    ) -> Result<GetDealTermReturn, ActorError> {
        rt.validate_immediate_caller_accept_any()?;
        let found = rt.state::<State>()?.get_proposal(rt.store(), params.id)?;
        Ok(GetDealTermReturn { start: found.start_epoch, duration: found.duration() })
    }

    /// Returns the total price that will be paid from the client to the provider for this deal.
    fn get_deal_total_price(
        rt: &impl Runtime,
        params: GetDealTotalPriceParams,
    ) -> Result<GetDealTotalPriceReturn, ActorError> {
        rt.validate_immediate_caller_accept_any()?;
        let found = rt.state::<State>()?.get_proposal(rt.store(), params.id)?;
        Ok(GetDealTotalPriceReturn { total_price: found.total_storage_fee() })
    }

    /// Returns the client collateral requirement for a deal proposal.
    fn get_deal_client_collateral(
        rt: &impl Runtime,
        params: GetDealClientCollateralParams,
    ) -> Result<GetDealClientCollateralReturn, ActorError> {
        rt.validate_immediate_caller_accept_any()?;
        let found = rt.state::<State>()?.get_proposal(rt.store(), params.id)?;
        Ok(GetDealClientCollateralReturn { collateral: found.client_collateral })
    }

    /// Returns the provider collateral requirement for a deal proposal.
    fn get_deal_provider_collateral(
        rt: &impl Runtime,
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
        rt: &impl Runtime,
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
        rt: &impl Runtime,
        params: GetDealActivationParams,
    ) -> Result<GetDealActivationReturn, ActorError> {
        rt.validate_immediate_caller_accept_any()?;
        let st = rt.state::<State>()?;
        let found = st.find_deal_state(rt.store(), params.id)?;
        match found {
            Some(state) => {
                if state.slash_epoch != EPOCH_UNDEFINED {
                    // Deal was terminated asynchronously
                    // TODO: https://github.com/filecoin-project/builtin-actors/issues/1388
                    Err(ActorError::unchecked(
                        EX_DEAL_EXPIRED,
                        format!("deal {} expired", params.id),
                    ))
                } else {
                    // If we have state, the deal has been activated
                    Ok(GetDealActivationReturn {
                        activated: state.sector_start_epoch,
                        terminated: state.slash_epoch,
                    })
                }
            }
            None => {
                // Pass through exit codes if proposal doesn't exist.
                let _ = st.get_proposal(rt.store(), params.id)?;
                // Proposal was published but never activated.
                Ok(GetDealActivationReturn {
                    activated: EPOCH_UNDEFINED,
                    terminated: EPOCH_UNDEFINED,
                })
            }
        }
    }

    /// Fetches the sector in which a deal is stored.
    /// This is available from after a deal is activated until it is finally settled
    /// (either normally or by termination).
    /// Fails with USR_NOT_FOUND if the deal doesn't exist (yet),
    /// EX_DEAL_NOT_ACTIVATED if the deal is published but has not been activated,
    /// or EX_DEAL_EXPIRED if the deal has been removed from state.
    fn get_deal_sector(
        rt: &impl Runtime,
        params: GetDealSectorParams,
    ) -> Result<GetDealSectorReturn, ActorError> {
        rt.validate_immediate_caller_accept_any()?;
        let st = rt.state::<State>()?;
        let found = st.find_deal_state(rt.store(), params.id)?;
        match found {
            Some(state) => {
                // The deal has been activated and not yet finally settled.
                if state.slash_epoch != EPOCH_UNDEFINED {
                    // The deal has been terminated but not cleaned up.
                    // Hide this internal state from caller and fail as if it had been cleaned up.
                    // This will become an impossible state when deal termination is
                    // processed immediately.
                    // Remove with https://github.com/filecoin-project/builtin-actors/issues/1388.
                    Err(ActorError::unchecked(
                        EX_DEAL_EXPIRED,
                        format!("deal {} expired", params.id),
                    ))
                } else {
                    Ok(GetDealSectorReturn { sector: state.sector_number })
                }
            }
            None => {
                // Pass through exit codes if proposal doesn't exist.
                let _ = st.get_proposal(rt.store(), params.id)?;
                // Proposal was published but never activated.
                Err(ActorError::unchecked(
                    EX_DEAL_NOT_ACTIVATED,
                    format!("deal {} not yet activated", params.id),
                ))
            }
        }
    }

    fn settle_deal_payments(
        rt: &impl Runtime,
        params: SettleDealPaymentsParams,
    ) -> Result<SettleDealPaymentsReturn, ActorError> {
        rt.validate_immediate_caller_accept_any()?;
        let curr_epoch = rt.curr_epoch();

        let mut batch_gen = BatchReturnGen::new(params.deal_ids.len() as usize);
        let mut settlements: Vec<DealSettlementSummary> = Vec::new();
        // accumulates slashed amounts from timed out deal proposals that weren't activated in time
        let mut total_slashed = TokenAmount::zero();

        rt.transaction(|st: &mut State, rt| {
            let mut new_deal_states: Vec<(DealID, DealState)> = Vec::new();
            let mut provider_deals_to_remove =
                BTreeMap::<ActorID, BTreeMap<SectorNumber, Vec<DealID>>>::new();
            for deal_id in params.deal_ids.iter() {
                let deal_proposal = match st.get_proposal(rt.store(), deal_id) {
                    Ok(prop) => prop,
                    Err(_) => {
                        batch_gen.add_fail(EX_DEAL_EXPIRED);
                        continue;
                    }
                };
                let dcid = match deal_cid(rt, &deal_proposal) {
                    Ok(cid) => cid,
                    Err(e) => {
                        batch_gen.add_fail(e.exit_code());
                        continue;
                    }
                };

                let loaded_deal = match st.get_active_deal_or_process_timeout(
                    rt.store(),
                    curr_epoch,
                    deal_id,
                    &deal_proposal,
                    &dcid,
                ) {
                    Ok(res) => res,
                    Err(e) => {
                        batch_gen.add_fail(e.exit_code());
                        continue;
                    }
                };

                let mut deal_state = match loaded_deal {
                    LoadDealState::TooEarly => {
                        // deal is not active, we process it as a zero-payment no-op
                        settlements.push(DealSettlementSummary {
                            completed: false,
                            payment: TokenAmount::zero(),
                        });
                        batch_gen.add_success();
                        continue;
                    }
                    LoadDealState::ProposalExpired(penalty) => {
                        // deal proposal was not activated in time
                        total_slashed += penalty;
                        batch_gen.add_fail(EX_DEAL_EXPIRED);
                        continue;
                    }
                    LoadDealState::Loaded(deal_state) => deal_state,
                };

                // TODO: remove this defensive check when it becomes impossible for process_deal_update to encounter slashed deals
                // https://github.com/filecoin-project/builtin-actors/issues/1388
                if deal_state.slash_epoch != EPOCH_UNDEFINED {
                    return Err(actor_error!(
                        illegal_argument,
                        "deal {} is marked for termination and cannot be settled",
                        deal_id
                    ));
                }

                let (_, payment_amount, completed, remove_deal) = match st.process_deal_update(
                    rt.store(),
                    &deal_state,
                    &deal_proposal,
                    &dcid,
                    curr_epoch,
                ) {
                    Ok(res) => res,
                    Err(e) => {
                        batch_gen.add_fail(e.exit_code());
                        continue;
                    }
                };

                if remove_deal {
                    st.remove_completed_deal(rt.store(), deal_id)?;
                    provider_deals_to_remove
                        .entry(deal_proposal.provider.id().unwrap())
                        .or_default()
                        .entry(deal_state.sector_number)
                        .or_default()
                        .push(deal_id);

                    if !completed {
                        emit::deal_terminated(
                            rt,
                            deal_id,
                            deal_proposal.client.id().unwrap(),
                            deal_proposal.provider.id().unwrap(),
                        )?;
                    }
                } else {
                    deal_state.last_updated_epoch = curr_epoch;
                    new_deal_states.push((deal_id, deal_state));
                }

                settlements.push(DealSettlementSummary {
                    completed: remove_deal,
                    payment: payment_amount,
                });
                batch_gen.add_success();

                if completed {
                    emit::deal_completed(
                        rt,
                        deal_id,
                        deal_proposal.client.id().unwrap(),
                        deal_proposal.provider.id().unwrap(),
                    )?;
                }
            }

            st.put_deal_states(rt.store(), &new_deal_states)?;
            st.remove_sector_deal_ids(rt.store(), &provider_deals_to_remove)?;
            Ok(())
        })?;

        if !total_slashed.is_zero() {
            extract_send_result(rt.send_simple(
                &BURNT_FUNDS_ACTOR_ADDR,
                METHOD_SEND,
                None,
                total_slashed,
            ))?;
        }

        Ok(SettleDealPaymentsReturn { results: batch_gen.gen(), settlements })
    }
}

fn get_proposals<BS: Blockstore>(
    proposal_array: &DealArray<BS>,
    deal_ids: &[DealID],
    next_id: DealID,
) -> Result<Vec<(DealID, DealProposal)>, ActorError> {
    let mut proposals = Vec::new();
    let mut seen_deal_ids = BTreeSet::new();
    for deal_id in deal_ids {
        if !seen_deal_ids.insert(deal_id) {
            return Err(actor_error!(illegal_argument, "duplicate deal ID {} in sector", deal_id));
        }
        let proposal = get_proposal(proposal_array, *deal_id, next_id)?;
        proposals.push((*deal_id, proposal));
    }
    Ok(proposals)
}

fn compute_data_commitment<'a>(
    rt: &impl Runtime,
    proposals: impl IntoIterator<Item = &'a DealProposal>,
    sector_type: RegisteredSealProof,
) -> Result<Cid, ActorError> {
    let mut pieces = vec![];

    for deal in proposals {
        pieces.push(PieceInfo { cid: deal.piece_cid, size: deal.piece_size });
    }

    rt.compute_unsealed_sector_cid(sector_type, &pieces).map_err(|e| {
        e.downcast_default(ExitCode::USR_ILLEGAL_ARGUMENT, "failed to compute unsealed sector CID")
    })
}

// Validates that each of a collection of deal proposals is valid and that they
// all fit within a sector.
pub fn validate_deals_for_sector(
    proposals: &[(DealID, DealProposal)],
    miner_addr: &Address,
    sector_expiry: ChainEpoch,
    sector_activation: ChainEpoch,
    sector_size: Option<SectorSize>,
) -> Result<(), ActorError> {
    let mut deal_space = BigInt::zero();
    let mut verified_deal_space = BigInt::zero();

    for (deal_id, proposal) in proposals {
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

    Ok(())
}

// Validates a deal is ready to activate now.
// There are two types of error possible here:
// - An Err in the outer result indicates something broken that should be propagated
//   and abort the current message.
// - An Err in the inner result indicates a problem with this deal, but not something that
//   ought to prevent other deals from being activated.
#[allow(clippy::too_many_arguments)]
fn preactivate_deal<BS: Blockstore>(
    rt: &impl Runtime,
    deal_id: DealID,
    proposals: &DealArray<BS>,
    states: &DealMetaArray<BS>,
    pending_proposals: &PendingProposalsSet<&BS>,
    provider: &Address,
    sector_commitment: ChainEpoch,
    curr_epoch: ChainEpoch,
    next_id: DealID,
) -> Result<Result<DealProposal, ActorError>, ActorError> {
    let proposal = match get_proposal(proposals, deal_id, next_id) {
        Ok(p) => p,
        Err(e) => {
            return match e.exit_code() {
                ExitCode::USR_NOT_FOUND | EX_DEAL_EXPIRED => Ok(Err(e)), // Fail this deal only.
                _ => Err(e),                                             // Abort.
            };
        }
    };

    let ok = validate_deal_can_activate(&proposal, provider, sector_commitment, curr_epoch);
    if let Err(e) = ok {
        return Ok(Err(e).with_context(|| format!("cannot activate deal {}", deal_id)));
    }

    if find_deal_state(states, deal_id)?.is_some() {
        return Ok(Err(actor_error!(illegal_argument, "deal {} already activated", deal_id)));
    }

    // Confirm the deal is in the pending proposals set.
    // It will be removed from this queue later, during cron.
    // Failing this check is an internal invariant violation.
    // The pending deals set exists to prevent duplicate proposals.
    // It should be impossible to have a proposal, no deal state, and not be in pending deals.
    let deal_cid = deal_cid(rt, &proposal)?;
    if !pending_proposals.has(&deal_cid)? {
        return Ok(Err(actor_error!(illegal_state, "deal {} is not in pending set", deal_cid)));
    }

    Ok(Ok(proposal))
}

fn alloc_request_for_deal(
    // Deal proposal must have ID addresses
    deal: &DealProposal,
    policy: &Policy,
    curr_epoch: ChainEpoch,
) -> ext::verifreg::AllocationRequest {
    let alloc_term_min = deal.end_epoch - deal.start_epoch;
    let alloc_term_max = min(
        alloc_term_min + policy.market_default_allocation_term_buffer,
        policy.maximum_verified_allocation_term,
    );
    let alloc_expiration =
        min(deal.start_epoch, curr_epoch + policy.maximum_verified_allocation_expiration);
    ext::verifreg::AllocationRequest {
        provider: deal.provider.id().unwrap(),
        data: deal.piece_cid,
        size: deal.piece_size,
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
    let datacap_required: u64 = alloc_reqs.iter().map(|it| it.size.0).sum();
    Ok(TransferFromParams {
        from: *client,
        to: VERIFIED_REGISTRY_ACTOR_ADDR,
        amount: TokenAmount::from_whole(datacap_required),
        operator_data: serialize(
            &ext::verifreg::AllocationRequests { allocations: alloc_reqs, extensions: vec![] },
            "allocation requests",
        )?,
    })
}

// Invokes transfer_from on the data cap token actor.
fn transfer_from(
    rt: &impl Runtime,
    params: TransferFromParams,
) -> Result<Vec<AllocationID>, ActorError> {
    let ret = extract_send_result(rt.send_simple(
        &DATACAP_TOKEN_ACTOR_ADDR,
        ext::datacap::TRANSFER_FROM_METHOD,
        IpldBlock::serialize_cbor(&params)?,
        TokenAmount::zero(),
    ))
    .context(format!("failed to send transfer to datacap {:?}", params))?;
    let ret: TransferFromReturn = ret
        .with_context_code(ExitCode::USR_ASSERTION_FAILED, || "return expected".to_string())?
        .deserialize()?;
    let allocs: ext::verifreg::AllocationsResponse =
        deserialize(&ret.recipient_data, "allocations response")?;
    Ok(allocs.new_allocations)
}

// Invokes BalanceOf on the data cap token actor.
fn balance_of(rt: &impl Runtime, owner: &Address) -> Result<TokenAmount, ActorError> {
    let params = IpldBlock::serialize_cbor(owner)?;
    let ret = extract_send_result(rt.send_simple(
        &DATACAP_TOKEN_ACTOR_ADDR,
        ext::datacap::BALANCE_OF_METHOD,
        params,
        TokenAmount::zero(),
    ))
    .context(format!("failed to query datacap balance of {}", owner))?;
    let ret: BalanceReturn = ret
        .with_context_code(ExitCode::USR_ASSERTION_FAILED, || "return expected".to_string())?
        .deserialize()?;
    Ok(ret)
}

// Calculates the first update epoch for a deal ID that is no sooner than `earliest`.
// An ID is processed as a fixed offset within each `interval` of epochs.
pub fn next_update_epoch(id: DealID, interval: i64, earliest: ChainEpoch) -> ChainEpoch {
    // Same logic as QuantSpec from the miner actor, but duplicated here to avoid unnecessary
    // dependencies.
    let offset = id as i64 % interval;
    let remainder = (earliest - offset) % interval;
    let quotient = (earliest - offset) / interval;

    // Don't round if epoch falls on a quantization epoch or when negative (negative truncating
    // division rounds up).
    if remainder == 0 || earliest - offset < 0 {
        interval * quotient + offset
    } else {
        interval * (quotient + 1) + offset
    }
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
        return Err(ActorError::forbidden(format!(
            "proposal has provider {}, must be {}",
            proposal.provider, miner_addr
        )));
    };

    if curr_epoch > proposal.start_epoch {
        return Err(ActorError::unchecked(
            // Use the same code as if the proposal had already been cleaned up from state.
            EX_DEAL_EXPIRED,
            format!(
                "proposal start epoch {} has already elapsed at {}",
                proposal.start_epoch, curr_epoch
            ),
        ));
    };

    if proposal.end_epoch > sector_expiration {
        return Err(ActorError::illegal_argument(format!(
            "proposal expiration {} exceeds sector expiration {}",
            proposal.end_epoch, sector_expiration
        )));
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
    let proposal_bytes = serialize(&proposal.proposal, "deal proposal")?;

    if !extract_send_result(rt.send(
        &proposal.proposal.client,
        ext::account::AUTHENTICATE_MESSAGE_METHOD,
        IpldBlock::serialize_cbor(&ext::account::AuthenticateMessageParams {
            signature: signature_bytes,
            message: proposal_bytes.to_vec(),
        })?,
        TokenAmount::zero(),
        None,
        SendFlags::READ_ONLY,
    ))
    .and_then(deserialize_block)
    .context("proposal authentication failed")?
    {
        Err(actor_error!(illegal_argument, "proposal authentication failed"))
    } else {
        Ok(())
    }
}

/// Compute a deal CID using the runtime.
pub fn deal_cid(rt: &impl Runtime, proposal: &DealProposal) -> Result<Cid, ActorError> {
    let data = serialize(proposal, "deal proposal")?;
    serialized_deal_cid(rt, data.bytes())
}

/// Compute a deal CID from serialized proposal using the runtime
pub(crate) fn serialized_deal_cid(rt: &impl Runtime, data: &[u8]) -> Result<Cid, ActorError> {
    const DIGEST_SIZE: u32 = 32;
    let hash = MultihashGeneric::wrap(Code::Blake2b256.into(), &rt.hash_blake2b(data))
        .map_err(|e| actor_error!(illegal_argument; "failed to take cid of proposal {}", e))?;
    debug_assert_eq!(u32::from(hash.size()), DIGEST_SIZE, "expected 32byte digest");
    Ok(Cid::new_v1(DAG_CBOR, hash))
}

fn request_miner_control_addrs(
    rt: &impl Runtime,
    miner_id: ActorID,
) -> Result<(Address, Address, Vec<Address>), ActorError> {
    let addrs: ext::miner::GetControlAddressesReturnParams =
        deserialize_block(extract_send_result(rt.send_simple(
            &Address::new_id(miner_id),
            ext::miner::CONTROL_ADDRESSES_METHOD,
            None,
            TokenAmount::zero(),
        ))?)?;

    Ok((addrs.owner, addrs.worker, addrs.control_addresses))
}

/// Resolves a provider or client address to the canonical form against which a balance should be held, and
/// the designated recipient address of withdrawals (which is the same, for simple account parties).
fn escrow_address(
    rt: &impl Runtime,
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
fn request_current_baseline_power(rt: &impl Runtime) -> Result<StoragePower, ActorError> {
    let ret: ThisEpochRewardReturn = deserialize_block(extract_send_result(rt.send_simple(
        &REWARD_ACTOR_ADDR,
        ext::reward::THIS_EPOCH_REWARD_METHOD,
        None,
        TokenAmount::zero(),
    ))?)?;
    Ok(ret.this_epoch_baseline_power)
}

/// Requests the current network total power and pledge from the power actor.
/// Returns a tuple of (raw_power, qa_power).
fn request_current_network_power(
    rt: &impl Runtime,
) -> Result<(StoragePower, StoragePower), ActorError> {
    let ret: ext::power::CurrentTotalPowerReturnParams =
        deserialize_block(extract_send_result(rt.send_simple(
            &STORAGE_POWER_ACTOR_ADDR,
            ext::power::CURRENT_TOTAL_POWER_METHOD,
            None,
            TokenAmount::zero(),
        ))?)?;
    Ok((ret.raw_byte_power, ret.quality_adj_power))
}

pub fn deal_id_key(k: DealID) -> BytesKey {
    let bz = k.encode_var_vec();
    bz.into()
}

pub fn sector_number_key(k: SectorNumber) -> BytesKey {
    let bz = k.encode_var_vec();
    bz.into()
}

impl ActorCode for Actor {
    type Methods = Method;

    fn name() -> &'static str {
        "StorageMarket"
    }

    actor_dispatch! {
        Constructor => constructor,
        AddBalance|AddBalanceExported => add_balance,
        WithdrawBalance|WithdrawBalanceExported => withdraw_balance,
        PublishStorageDeals|PublishStorageDealsExported => publish_storage_deals,
        VerifyDealsForActivation => verify_deals_for_activation,
        BatchActivateDeals => batch_activate_deals,
        OnMinerSectorsTerminate => on_miner_sectors_terminate,
        CronTick => cron_tick,
        GetBalanceExported => get_balance,
        GetDealDataCommitmentExported => get_deal_data_commitment,
        GetDealClientExported => get_deal_client,
        GetDealProviderExported => get_deal_provider,
        GetDealLabelExported => get_deal_label,
        GetDealTermExported => get_deal_term,
        GetDealTotalPriceExported => get_deal_total_price,
        GetDealClientCollateralExported => get_deal_client_collateral,
        GetDealProviderCollateralExported => get_deal_provider_collateral,
        GetDealVerifiedExported => get_deal_verified,
        GetDealActivationExported => get_deal_activation,
        GetDealSectorExported => get_deal_sector,
        SettleDealPaymentsExported => settle_deal_payments,
        SectorContentChangedExported => sector_content_changed,
    }
}
