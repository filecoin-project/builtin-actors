// Copyright 2019-2022 ChainSafe Systems
// SPDX-License-Identifier: Apache-2.0, MIT

use fil_fungible_token::receiver::types::{
    FRC46TokenReceived, UniversalReceiverParams, FRC46_TOKEN_TYPE,
};
use fil_fungible_token::token::types::{BurnParams, BurnReturn, TransferParams};
use fil_fungible_token::token::TOKEN_PRECISION;
use fvm_ipld_blockstore::Blockstore;
use fvm_ipld_encoding::RawBytes;
use fvm_ipld_hamt::BytesKey;
use fvm_shared::address::Address;
use fvm_shared::bigint::bigint_ser::BigIntDe;
use fvm_shared::bigint::BigInt;
use fvm_shared::clock::ChainEpoch;
use fvm_shared::econ::TokenAmount;
use fvm_shared::error::ExitCode;
use fvm_shared::{ActorID, MethodNum, HAMT_BIT_WIDTH, METHOD_CONSTRUCTOR};
use log::info;
use num_derive::FromPrimitive;
use num_traits::{FromPrimitive, Signed, Zero};

use fil_actors_runtime::cbor::{deserialize, serialize};
use fil_actors_runtime::runtime::builtins::Type;
use fil_actors_runtime::runtime::{ActorCode, Policy, Runtime};
use fil_actors_runtime::{
    actor_error, cbor, make_map_with_root_and_bitwidth, resolve_to_actor_id, ActorDowncast,
    ActorError, Map, STORAGE_MARKET_ACTOR_ADDR, SYSTEM_ACTOR_ADDR,
    UNIVERSAL_RECEIVER_HOOK_METHOD_NUM,
};

use crate::ext::datacap::{DestroyParams, MintParams, TOKEN_PRECISION};

pub use self::state::Allocation;
pub use self::state::Claim;
pub use self::state::State;
pub use self::types::*;

#[cfg(feature = "fil-actor")]
fil_actors_runtime::wasm_trampoline!(Actor);

pub mod ext;
pub mod state;
pub mod testing;
pub mod types;

/// Account actor methods available
#[derive(FromPrimitive)]
#[repr(u64)]
pub enum Method {
    Constructor = METHOD_CONSTRUCTOR,
    AddVerifier = 2,
    RemoveVerifier = 3,
    AddVerifiedClient = 4,
    UseBytes = 5,     // Deprecated
    RestoreBytes = 6, // Deprecated
    RemoveVerifiedClientDataCap = 7,
    RemoveExpiredAllocations = 8,
    ClaimAllocations = 9,
    UniversalReceiverHook = UNIVERSAL_RECEIVER_HOOK_METHOD_NUM,
}

pub struct Actor;

impl Actor {
    /// Constructor for Registry Actor
    pub fn constructor<BS, RT>(rt: &mut RT, root_key: Address) -> Result<(), ActorError>
    where
        BS: Blockstore,
        RT: Runtime<BS>,
    {
        rt.validate_immediate_caller_is(std::iter::once(&*SYSTEM_ACTOR_ADDR))?;

        // root should be an ID address
        let id_addr = rt
            .resolve_address(&root_key)
            .context_code(ExitCode::USR_ILLEGAL_ARGUMENT, "root should be an ID address")?;

        let st = State::new(rt.store(), Address::new_id(id_addr))
            .context("failed to create verifreg state")?;

        rt.create(&st)?;
        Ok(())
    }

    pub fn add_verifier<BS, RT>(rt: &mut RT, params: AddVerifierParams) -> Result<(), ActorError>
    where
        BS: Blockstore,
        RT: Runtime<BS>,
    {
        if params.allowance < rt.policy().minimum_verified_allocation_size {
            return Err(actor_error!(
                illegal_argument,
                "Allowance {} below minimum deal size for add verifier {}",
                params.allowance,
                params.address
            ));
        }

        let verifier = resolve_to_actor_id(rt, &params.address).map_err(|e| {
            e.downcast_default(
                ExitCode::USR_ILLEGAL_STATE,
                format!("failed to resolve addr {} to ID", params.address),
            )
        })?;

        let verifier = Address::new_id(verifier);

        let st: State = rt.state()?;
        rt.validate_immediate_caller_is(std::iter::once(&st.root_key))?;

        // Disallow root as a verifier.
        if verifier == st.root_key {
            return Err(actor_error!(illegal_argument, "Rootkey cannot be added as verifier"));
        }

        // Disallow existing clients as verifiers.
        let token_balance = balance_of(rt, &verifier)?;
        if token_balance.is_positive() {
            return Err(actor_error!(
                illegal_argument,
                "verified client {} cannot become a verifier",
                verifier
            ));
        }

        // Store the new verifier and allowance (over-writing).
        rt.transaction(|st: &mut State, rt| {
            st.put_verifier(rt.store(), &verifier, &params.allowance)
                .context("failed to add verifier")
        })
    }

    pub fn remove_verifier<BS, RT>(rt: &mut RT, verifier_addr: Address) -> Result<(), ActorError>
    where
        BS: Blockstore,
        RT: Runtime<BS>,
    {
        let verifier = resolve_to_actor_id(rt, &verifier_addr).map_err(|e| {
            e.downcast_default(
                ExitCode::USR_ILLEGAL_STATE,
                format!("failed to resolve addr {} to ID", verifier_addr),
            )
        })?;

        let verifier = Address::new_id(verifier);

        let state: State = rt.state()?;
        rt.validate_immediate_caller_is(std::iter::once(&state.root_key))?;

        rt.transaction(|st: &mut State, rt| {
            st.remove_verifier(rt.store(), &verifier).context("failed to remove verifier")
        })
    }

    pub fn add_verified_client<BS, RT>(
        rt: &mut RT,
        params: AddVerifierClientParams,
    ) -> Result<(), ActorError>
    where
        BS: Blockstore,
        RT: Runtime<BS>,
    {
        // The caller will be verified by checking table below
        rt.validate_immediate_caller_accept_any()?;

        if params.allowance < rt.policy().minimum_verified_allocation_size {
            return Err(actor_error!(
                illegal_argument,
                "allowance {} below MinVerifiedDealSize for add verified client {}",
                params.allowance,
                params.address
            ));
        }

        let client = resolve_to_actor_id(rt, &params.address).map_err(|e| {
            e.downcast_default(
                ExitCode::USR_ILLEGAL_STATE,
                format!("failed to resolve addr {} to ID", params.address),
            )
        })?;

        let client = Address::new_id(client);

        let st: State = rt.state()?;
        if client == st.root_key {
            return Err(actor_error!(illegal_argument, "root cannot be added as client"));
        }

        // Validate caller is one of the verifiers, i.e. has an allowance (even if zero).
        let verifier = rt.message().caller();
        let verifier_cap = st
            .get_verifier_cap(rt.store(), &verifier)?
            .ok_or_else(|| actor_error!(not_found, "caller {} is not a verifier", verifier))?;

        // Disallow existing verifiers as clients.
        if st.get_verifier_cap(rt.store(), &client)?.is_some() {
            return Err(actor_error!(
                illegal_argument,
                "verifier {} cannot be added as a verified client",
                client
            ));
        }

        // Compute new verifier allowance.
        if verifier_cap < params.allowance {
            return Err(actor_error!(
                illegal_argument,
                "add more DataCap {} for client than allocated {}",
                params.allowance,
                verifier_cap
            ));
        }

        // Reduce verifier's cap.
        let new_verifier_cap = verifier_cap - &params.allowance;
        rt.transaction(|st: &mut State, rt| {
            st.put_verifier(rt.store(), &verifier, &new_verifier_cap)
                .context("failed to update verifier allowance")
        })?;

        // Credit client token allowance.
        let operators = vec![*STORAGE_MARKET_ACTOR_ADDR];
        mint(rt, &client, &params.allowance, operators).context(format!(
            "failed to mint {} data cap to client {}",
            &params.allowance, client
        ))?;
        Ok(())
    }

    /// Called by StorageMarketActor during PublishStorageDeals.
    /// Do not allow partially verified deals (DealSize must be greater than equal to allowed cap).
    /// Delete VerifiedClient if remaining DataCap is smaller than minimum VerifiedDealSize.
    pub fn use_bytes<BS, RT>(rt: &mut RT, params: UseBytesParams) -> Result<(), ActorError>
    where
        BS: Blockstore,
        RT: Runtime<BS>,
    {
        rt.validate_immediate_caller_is(std::iter::once(&*STORAGE_MARKET_ACTOR_ADDR))?;

        let client = resolve_to_actor_id(rt, &params.address).map_err(|e| {
            e.downcast_default(
                ExitCode::USR_ILLEGAL_STATE,
                format!("failed to resolve addr {} to ID", params.address),
            )
        })?;

        let client = Address::new_id(client);

        if params.deal_size < rt.policy().minimum_verified_allocation_size {
            return Err(actor_error!(
                illegal_argument,
                "use bytes {} is below minimum {}",
                params.deal_size,
                rt.policy().minimum_verified_allocation_size
            ));
        }

        // Deduct from client's token allowance.
        let remaining = destroy(rt, &client, &params.deal_size).context(format!(
            "failed to deduct {} from allowance for {}",
            &params.deal_size, &client
        ))?;

        // Destroy any remaining balance below minimum verified deal size.
        if remaining.is_positive() && remaining < rt.policy().minimum_verified_allocation_size {
            destroy(rt, &client, &remaining).context(format!(
                "failed to destroy remaining {} from allowance for {}",
                &remaining, &client
            ))?;
        }
        Ok(())
    }

    /// Called by HandleInitTimeoutDeals from StorageMarketActor when a VerifiedDeal fails to init.
    /// Restore allowable cap for the client, creating new entry if the client has been deleted.
    pub fn restore_bytes<BS, RT>(rt: &mut RT, params: RestoreBytesParams) -> Result<(), ActorError>
    where
        BS: Blockstore,
        RT: Runtime<BS>,
    {
        rt.validate_immediate_caller_is(std::iter::once(&*STORAGE_MARKET_ACTOR_ADDR))?;
        if params.deal_size < rt.policy().minimum_verified_allocation_size {
            return Err(actor_error!(
                illegal_argument,
                "Below minimum VerifiedDealSize requested in RestoreBytes: {}",
                params.deal_size
            ));
        }

        let client = resolve_to_actor_id(rt, &params.address).map_err(|e| {
            e.downcast_default(
                ExitCode::USR_ILLEGAL_STATE,
                format!("failed to resolve addr {} to ID", params.address),
            )
        })?;

        let client = Address::new_id(client);

        let st: State = rt.state()?;
        // Disallow root as a client.
        if client == st.root_key {
            return Err(actor_error!(illegal_argument, "cannot restore allowance for root"));
        }

        // Disallow existing verifiers as clients.
        if st.get_verifier_cap(rt.store(), &client)?.is_some() {
            return Err(actor_error!(
                illegal_argument,
                "cannot restore allowance for verifier {}",
                client
            ));
        }

        let operators = vec![*STORAGE_MARKET_ACTOR_ADDR];
        mint(rt, &client, &params.deal_size, operators).context(format!(
            "failed to restore {} to allowance for {}",
            &params.deal_size, &client
        ))
    }

    /// Removes DataCap allocated to a verified client.
    pub fn remove_verified_client_data_cap<BS, RT>(
        rt: &mut RT,
        params: RemoveDataCapParams,
    ) -> Result<RemoveDataCapReturn, ActorError>
    where
        BS: Blockstore,
        RT: Runtime<BS>,
    {
        let client = resolve_to_actor_id(rt, &params.verified_client_to_remove).map_err(|e| {
            e.downcast_default(
                ExitCode::USR_ILLEGAL_ARGUMENT,
                format!("failed to resolve client addr {} to ID", params.verified_client_to_remove),
            )
        })?;

        let client = Address::new_id(client);

        let verifier_1 =
            resolve_to_actor_id(rt, &params.verifier_request_1.verifier).map_err(|e| {
                e.downcast_default(
                    ExitCode::USR_ILLEGAL_ARGUMENT,
                    format!(
                        "failed to resolve verifier addr {} to ID",
                        params.verifier_request_1.verifier
                    ),
                )
            })?;

        let verifier_1 = Address::new_id(verifier_1);

        let verifier_2 =
            resolve_to_actor_id(rt, &params.verifier_request_2.verifier).map_err(|e| {
                e.downcast_default(
                    ExitCode::USR_ILLEGAL_ARGUMENT,
                    format!(
                        "failed to resolve verifier addr {} to ID",
                        params.verifier_request_2.verifier
                    ),
                )
            })?;

        let verifier_2 = Address::new_id(verifier_2);

        if verifier_1 == verifier_2 {
            return Err(actor_error!(
                illegal_argument,
                "need two different verifiers to send remove datacap request"
            ));
        }

        // Validate and then remove the proposal.
        rt.transaction(|st: &mut State, rt| {
            rt.validate_immediate_caller_is(std::iter::once(&st.root_key))?;

            if !is_verifier(rt, st, verifier_1)? {
                return Err(actor_error!(not_found, "{} is not a verifier", verifier_1));
            }

            if !is_verifier(rt, st, verifier_2)? {
                return Err(actor_error!(not_found, "{} is not a verifier", verifier_2));
            }

            // validate signatures
            let mut proposal_ids = make_map_with_root_and_bitwidth::<_, RemoveDataCapProposalID>(
                &st.remove_data_cap_proposal_ids,
                rt.store(),
                HAMT_BIT_WIDTH,
            )
            .map_err(|e| {
                e.downcast_default(
                    ExitCode::USR_ILLEGAL_STATE,
                    "failed to load datacap removal proposal ids",
                )
            })?;

            let verifier_1_id = use_proposal_id(&mut proposal_ids, verifier_1, client)?;
            let verifier_2_id = use_proposal_id(&mut proposal_ids, verifier_2, client)?;

            remove_data_cap_request_is_valid(
                rt,
                &params.verifier_request_1,
                verifier_1_id,
                &params.data_cap_amount_to_remove,
                client,
            )?;
            remove_data_cap_request_is_valid(
                rt,
                &params.verifier_request_2,
                verifier_2_id,
                &params.data_cap_amount_to_remove,
                client,
            )?;

            st.remove_data_cap_proposal_ids = proposal_ids
                .flush()
                .context_code(ExitCode::USR_ILLEGAL_STATE, "failed to flush proposal ids")?;
            Ok(())
        })?;

        // Burn the client's data cap tokens.
        let balance = balance_of(rt, &client).context("failed to fetch balance")?;
        let burnt = std::cmp::min(balance, params.data_cap_amount_to_remove);
        destroy(rt, &client, &burnt)
            .context(format!("failed to destroy {} from allowance for {}", &burnt, &client))?;

        Ok(RemoveDataCapReturn {
            verified_client: client, // Changed to the resolved address
            data_cap_removed: burnt,
        })
    }

    // An allocation may be removed after its expiration epoch has passed (by anyone).
    // When removed, the DataCap tokens are transferred back to the client.
    pub fn remove_expired_allocations<BS, RT>(
        rt: &mut RT,
        params: RemoveExpiredAllocationsParams,
    ) -> Result<RemoveExpiredAllocationsReturn, ActorError>
    where
        BS: Blockstore,
        RT: Runtime<BS>,
    {
        // since the alloc is expired this should be safe to publically cleanup
        rt.validate_immediate_caller_accept_any()?;
        let client = params.client;
        let mut ret_gen = BatchReturnGen::new(params.allocation_ids.len());
        let mut recovered_datacap = DataCap::zero();
        rt.transaction(|st: &mut State, rt| {
            let mut allocs = st.load_allocs(rt.store())?;
            for alloc_id in params.allocation_ids {
                let maybe_alloc = allocs.get(client, alloc_id).context_code(
                    ExitCode::USR_ILLEGAL_STATE,
                    "HAMT lookup failure getting allocation",
                )?;
                let alloc = match maybe_alloc {
                    None => {
                        ret_gen.add_fail(ExitCode::USR_NOT_FOUND);
                        info!(
                            "claim references allocation id {} that does not belong to client {}",
                            alloc_id, client,
                        );
                        continue;
                    }
                    Some(a) => a,
                };
                if alloc.expiration > rt.curr_epoch() {
                    ret_gen.add_fail(ExitCode::USR_FORBIDDEN);
                    info!("cannot revoke allocation {} that has not expired", alloc_id);
                    continue;
                }
                recovered_datacap += alloc.size.0;

                allocs.remove(client, alloc_id).context_code(
                    ExitCode::USR_ILLEGAL_STATE,
                    format!("failed to remove allocation {}", alloc_id),
                )?;
                ret_gen.add_success();
            }
            st.save_allocs(&mut allocs)?;
            Ok(())
        })
        .context("state transaction failed")?;

        // Transfer the recovered datacap back to the client.
        transfer(rt, &client, &recovered_datacap).with_context(|| {
            format!(
                "failed to transfer recovered datacap {} back to client {}",
                &recovered_datacap, &client
            )
        })?;

        Ok(ret_gen.gen())
    }

    // Called by storage provider actor to claim allocations for data provably committed to storage.
    // For each allocation claim, the registry checks that the provided piece CID
    // and size match that of the allocation.
    pub fn claim_allocations<BS, RT>(
        rt: &mut RT,
        params: ClaimAllocationsParams,
    ) -> Result<ClaimAllocationsReturn, ActorError>
    where
        BS: Blockstore,
        RT: Runtime<BS>,
    {
        rt.validate_immediate_caller_type(std::iter::once(&Type::Miner))?;
        let provider = rt.message().caller();
        if params.sectors.is_empty() {
            return Err(actor_error!(illegal_argument, "claim allocations called with no claims"));
        }
        let mut datacap_claimed = DataCap::zero();
        let mut ret_gen = BatchReturnGen::new(params.sectors.len());
        rt.transaction(|st: &mut State, rt| {
            let mut claims = st.load_claims(rt.store())?;
            let mut allocs = st.load_allocs(rt.store())?;

            for claim_alloc in params.sectors {
                let maybe_alloc =
                    allocs.get(claim_alloc.client, claim_alloc.allocation_id).context_code(
                        ExitCode::USR_ILLEGAL_STATE,
                        "HAMT lookup failure getting allocation",
                    )?;

                let alloc: &Allocation = match maybe_alloc {
                    None => {
                        ret_gen.add_fail(ExitCode::USR_NOT_FOUND);
                        info!(
                            "no allocation {} for client {}",
                            claim_alloc.allocation_id, claim_alloc.client,
                        );
                        continue;
                    }
                    Some(a) => a,
                };

                if !can_claim_alloc(&claim_alloc, provider, alloc, rt.curr_epoch()) {
                    ret_gen.add_fail(ExitCode::USR_FORBIDDEN);
                    info!(
                        "invalid sector {:?} for allocation {}",
                        claim_alloc.sector, claim_alloc.allocation_id,
                    );
                    continue;
                }

                let new_claim = Claim {
                    provider,
                    client: alloc.client,
                    data: alloc.data,
                    size: alloc.size,
                    term_min: alloc.term_min,
                    term_max: alloc.term_max,
                    term_start: rt.curr_epoch(),
                    sector: claim_alloc.sector,
                };

                let inserted = claims
                    .put_if_absent(provider, claim_alloc.allocation_id, new_claim)
                    .context_code(
                        ExitCode::USR_ILLEGAL_STATE,
                        format!("failed to write claim {}", claim_alloc.allocation_id),
                    )?;
                if !inserted {
                    ret_gen.add_fail(ExitCode::USR_ILLEGAL_STATE); // should be unreachable since claim and alloc can't exist at once
                    info!(
                        "claim for allocation {} could not be inserted as it already exists",
                        claim_alloc.allocation_id,
                    );
                    continue;
                }

                allocs.remove(claim_alloc.client, claim_alloc.allocation_id).context_code(
                    ExitCode::USR_ILLEGAL_STATE,
                    format!("failed to remove allocation {}", claim_alloc.allocation_id),
                )?;

                datacap_claimed += DataCap::from(claim_alloc.size.0);
                ret_gen.add_success();
            }
            st.save_allocs(&mut allocs)?;
            st.save_claims(&mut claims)?;
            Ok(())
        })
        .context("state transaction failed")?;

        // Burn the datacap tokens from verified registry's own balance.
        burn(rt, &datacap_claimed)?;

        Ok(ret_gen.gen())
    }

    pub fn universal_receiver_hook<BS, RT>(
        rt: &mut RT,
        params: UniversalReceiverParams,
    ) -> Result<(), ActorError>
    where
        BS: Blockstore,
        RT: Runtime<BS>,
    {
        // Accept only the data cap token.
        rt.validate_immediate_caller_is(&[*DATACAP_TOKEN_ACTOR_ADDR])?;

        let my_id = rt.message().receiver().id().unwrap();
        let curr_epoch = rt.curr_epoch();

        // Validate receiver hook payload.
        let tokens_received = validate_tokens_received(&params, my_id)?;
        let token_datacap = tokens_to_datacap(&tokens_received.amount);
        let client_address = Address::new_id(tokens_received.from);

        // Extract and validate allocation request from the operator data.
        let alloc_reqs = validate_alloc_req(
            &tokens_received.operator_data,
            rt.policy(),
            curr_epoch,
            &token_datacap,
        )?;

        // Construct new allocation records.
        let mut new_allocs = Vec::with_capacity(alloc_reqs.requests.len());
        for req in alloc_reqs.requests {
            // Require the provider to be a miner actor.
            // This doesn't matter much, but is more ergonomic to fail rather than lock up datacap.
            let provider_id = resolve_miner_id(rt, &req.provider)?;
            new_allocs.push(Allocation {
                client: client_address,
                provider: Address::new_id(provider_id),
                data: req.data,
                size: req.size,
                term_min: req.term_min,
                term_max: req.term_max,
                expiration: req.expiration,
            })
        }
        // Save allocations
        rt.transaction(|st: &mut State, rt| {
            st.insert_allocations(rt.store(), &client_address, new_allocs.into_iter())
        })?;
        Ok(())
    }
}

// Checks whether an address has a verifier entry (which could be zero).
fn is_verifier<BS, RT>(rt: &RT, st: &State, address: Address) -> Result<bool, ActorError>
where
    BS: Blockstore,
    RT: Runtime<BS>,
{
    let verifiers =
        make_map_with_root_and_bitwidth::<_, BigIntDe>(&st.verifiers, rt.store(), HAMT_BIT_WIDTH)
            .context_code(ExitCode::USR_ILLEGAL_STATE, "failed to load verifiers")?;

    // check that the `address` is currently a verified client
    let found = verifiers
        .contains_key(&address.to_bytes())
        .context_code(ExitCode::USR_ILLEGAL_STATE, "failed to get verifier")?;

    Ok(found)
}

// Invokes BalanceOf on the data cap token actor, and converts the result to whole units of data cap.
fn balance_of<BS, RT>(rt: &mut RT, owner: &Address) -> Result<DataCap, ActorError>
where
    BS: Blockstore,
    RT: Runtime<BS>,
{
    let params = serialize(owner, "owner address")?;
    let ret = rt
        .send(
            &DATACAP_TOKEN_ACTOR_ADDR,
            ext::datacap::Method::BalanceOf as u64,
            params,
            TokenAmount::zero(),
        )
        .context(format!("failed to query datacap balance of {}", owner))?;
    let x: TokenAmount = deserialize(&ret, "balance result")?;
    Ok(tokens_to_datacap(&x))
}

// Invokes Mint on a data cap token actor for whole units of data cap.
fn mint<BS, RT>(
    rt: &mut RT,
    to: &Address,
    amount: &DataCap,
    operators: Vec<Address>,
) -> Result<(), ActorError>
where
    BS: Blockstore,
    RT: Runtime<BS>,
{
    let token_amt = datacap_to_tokens(amount);
    let params = MintParams { to: *to, amount: token_amt, operators };
    rt.send(
        &DATACAP_TOKEN_ACTOR_ADDR,
        ext::datacap::Method::Mint as u64,
        serialize(&params, "mint params")?,
        TokenAmount::zero(),
    )
    .context(format!("failed to send mint {:?} to datacap", params))?;
    Ok(())
}

// Invokes Burn on a data cap token actor for whole units of data cap.
fn burn<BS, RT>(rt: &mut RT, amount: &DataCap) -> Result<DataCap, ActorError>
where
    BS: Blockstore,
    RT: Runtime<BS>,
{
    let token_amt = datacap_to_tokens(amount);
    let params = BurnParams { amount: token_amt };
    let ret: BurnReturn = rt
        .send(
            &DATACAP_TOKEN_ACTOR_ADDR,
            ext::datacap::Method::Burn as u64,
            serialize(&params, "burn params")?,
            TokenAmount::zero(),
        )
        .context(format!("failed to send burn {:?} to datacap", params))?
        .deserialize()?;
    Ok(tokens_to_datacap(&ret.balance))
}

// Invokes Destroy on a data cap token actor for whole units of data cap.
fn destroy<BS, RT>(rt: &mut RT, owner: &Address, amount: &DataCap) -> Result<DataCap, ActorError>
where
    BS: Blockstore,
    RT: Runtime<BS>,
{
    let token_amt = datacap_to_tokens(amount);
    let params = DestroyParams { owner: *owner, amount: token_amt };
    let ret: BurnReturn = rt
        .send(
            &DATACAP_TOKEN_ACTOR_ADDR,
            ext::datacap::Method::Destroy as u64,
            serialize(&params, "destroy params")?,
            TokenAmount::zero(),
        )
        .context(format!("failed to send destroy {:?} to datacap", params))?
        .deserialize()?;
    Ok(tokens_to_datacap(&ret.balance))
}

// Invokes transfer on a data cap token actor for whole units of data cap.
fn transfer<BS, RT>(rt: &mut RT, to: &Address, amount: &DataCap) -> Result<(), ActorError>
where
    BS: Blockstore,
    RT: Runtime<BS>,
{
    let token_amt = datacap_to_tokens(amount);
    let params = TransferParams { to: *to, amount: token_amt, operator_data: Default::default() };
    rt.send(
        &DATACAP_TOKEN_ACTOR_ADDR,
        ext::datacap::Method::Transfer as u64,
        serialize(&params, "transfer params")?,
        TokenAmount::zero(),
    )
    .context(format!("failed to send transfer to datacap {:?}", params))?;
    Ok(())
}

fn datacap_to_tokens(amount: &DataCap) -> TokenAmount {
    amount * TOKEN_PRECISION
}

fn tokens_to_datacap(amount: &BigInt) -> BigInt {
    amount / TOKEN_PRECISION
}

fn use_proposal_id<BS>(
    proposal_ids: &mut Map<BS, RemoveDataCapProposalID>,
    verifier: Address,
    client: Address,
) -> Result<RemoveDataCapProposalID, ActorError>
where
    BS: Blockstore,
{
    let key = AddrPairKey::new(verifier, client);

    let maybe_id = proposal_ids.get(&key.to_bytes()).map_err(|e| {
        actor_error!(
            illegal_state,
            "failed to get proposal id for verifier {} and client {}: {}",
            verifier,
            client,
            e
        )
    })?;

    let curr_id = if let Some(RemoveDataCapProposalID { id }) = maybe_id {
        RemoveDataCapProposalID { id: *id }
    } else {
        RemoveDataCapProposalID { id: 0 }
    };

    let next_id = RemoveDataCapProposalID { id: curr_id.id + 1 };
    proposal_ids.set(BytesKey::from(key.to_bytes()), next_id).map_err(|e| {
        actor_error!(
            illegal_state,
            "failed to update proposal id for verifier {} and client {}: {}",
            verifier,
            client,
            e
        )
    })?;

    Ok(curr_id)
}

fn remove_data_cap_request_is_valid<BS, RT>(
    rt: &RT,
    request: &RemoveDataCapRequest,
    id: RemoveDataCapProposalID,
    to_remove: &DataCap,
    client: Address,
) -> Result<(), ActorError>
where
    BS: Blockstore,
    RT: Runtime<BS>,
{
    let proposal = RemoveDataCapProposal {
        removal_proposal_id: id,
        data_cap_amount: to_remove.clone(),
        verified_client: client,
    };

    let b = RawBytes::serialize(proposal).map_err(|e| {
        actor_error!(
                serialization; "failed to marshal remove datacap request: {}", e)
    })?;

    let payload = [SIGNATURE_DOMAIN_SEPARATION_REMOVE_DATA_CAP, b.bytes()].concat();

    // verify signature of proposal
    rt.verify_signature(&request.signature, &request.verifier, &payload).map_err(
        |e| actor_error!(illegal_argument; "invalid signature for datacap removal request: {}", e),
    )
}

// Deserializes and validates a receiver hook payload, expecting only an FRC-46 transfer.
fn validate_tokens_received(
    params: &UniversalReceiverParams,
    my_id: u64,
) -> Result<FRC46TokenReceived, ActorError> {
    if params.type_ != FRC46_TOKEN_TYPE {
        return Err(actor_error!(
            illegal_argument,
            "invalid token type {}, expected {} (FRC-46)",
            params.type_,
            FRC46_TOKEN_TYPE
        ));
    }
    let payload: FRC46TokenReceived = deserialize(&params.payload, "receiver hook payload")?;
    // Payload to address must match receiving actor.
    if payload.to != my_id {
        return Err(actor_error!(
            illegal_argument,
            "token receiver expected to {}, was {}",
            my_id,
            payload.to
        ));
    }
    Ok(payload)
}

// Deserializes and validates a serialized allocation.
fn validate_alloc_req(
    serialized: &RawBytes,
    policy: &Policy,
    curr_epoch: ChainEpoch,
    datacap_amount: &DataCap,
) -> Result<AllocationRequests, ActorError> {
    let reqs: AllocationRequests = deserialize(serialized, "allocation requests")?;

    let mut allocation_total = DataCap::zero();
    for req in &reqs.requests {
        allocation_total += DataCap::from(req.size.0);

        // Size must be at least the policy minimum.
        if DataCap::from(req.size.0) < policy.minimum_verified_allocation_size {
            return Err(actor_error!(
                illegal_argument,
                "allocation size {} below minimum {}",
                req.size.0,
                policy.minimum_verified_allocation_size
            ));
        }

        // Term must be at least the policy minimum.
        if req.term_min < policy.minimum_verified_allocation_term {
            return Err(actor_error!(
                illegal_argument,
                "allocation term min {} below limit {}",
                req.term_min,
                policy.minimum_verified_allocation_term
            ));
        }
        // Term cannot exceed the policy maximum.
        if req.term_max > policy.maximum_verified_allocation_term {
            return Err(actor_error!(
                illegal_argument,
                "allocation term max {} above limit {}",
                req.term_max,
                policy.maximum_verified_allocation_term
            ));
        }
        // Term range must be non-empty.
        if req.term_min > req.term_max {
            return Err(actor_error!(
                illegal_argument,
                "allocation term min {} exceeds term max {}",
                req.term_min,
                req.term_max
            ));
        }

        // Allocation must expire in the future.
        if req.expiration < curr_epoch {
            return Err(actor_error!(
                illegal_argument,
                "allocation expiration epoch {} has passed current epoch {}",
                req.expiration,
                curr_epoch
            ));
        }
        // Allocation must expire soon enough.
        let max_expiration = curr_epoch + policy.maximum_verified_allocation_expiration;
        if req.expiration > max_expiration {
            return Err(actor_error!(
                illegal_argument,
                "allocation expiration {} exceeds maximum {}",
                req.expiration,
                max_expiration
            ));
        }
    }

    // Allocation size must match the tokens received exactly (we can't return change).
    if &allocation_total != datacap_amount {
        return Err(actor_error!(
            illegal_argument,
            "total allocation size {} must match data cap amount received {}",
            allocation_total,
            datacap_amount
        ));
    }

    Ok(reqs)
}

// Checks that an address corresponsds to a miner actor.
fn resolve_miner_id<BS, RT>(rt: &mut RT, addr: &Address) -> Result<ActorID, ActorError>
where
    BS: Blockstore,
    RT: Runtime<BS>,
{
    let id = rt.resolve_address(addr).with_context_code(ExitCode::USR_ILLEGAL_ARGUMENT, || {
        format!("failed to resolve provider address {}", addr)
    })?;
    let code_cid =
        rt.get_actor_code_cid(&id).with_context_code(ExitCode::USR_ILLEGAL_ARGUMENT, || {
            format!("no code CID for provider {}", addr)
        })?;
    let provider_type = rt
        .resolve_builtin_actor_type(&code_cid)
        .with_context_code(ExitCode::USR_ILLEGAL_ARGUMENT, || {
            format!("provider code {} must be built-in miner actor", code_cid)
        })?;
    if provider_type != Type::Miner {
        return Err(actor_error!(
            illegal_argument,
            "allocation provider {} must be a miner actor, was {:?}",
            addr,
            provider_type
        ));
    }
    Ok(id)
}

fn can_claim_alloc(
    claim_alloc: &SectorAllocationClaim,
    provider: Address,
    alloc: &Allocation,
    curr_epoch: ChainEpoch,
) -> bool {
    let sector_lifetime = claim_alloc.sector_expiry - curr_epoch;

    provider == alloc.provider
        && claim_alloc.client == alloc.client
        && claim_alloc.data == alloc.data
        && claim_alloc.size == alloc.size
        && curr_epoch < alloc.expiration
        && sector_lifetime >= alloc.term_min
        && sector_lifetime <= alloc.term_max
}

impl ActorCode for Actor {
    fn invoke_method<BS, RT>(
        rt: &mut RT,
        method: MethodNum,
        params: &RawBytes,
    ) -> Result<RawBytes, ActorError>
    where
        BS: Blockstore,
        RT: Runtime<BS>,
    {
        match FromPrimitive::from_u64(method) {
            Some(Method::Constructor) => {
                Self::constructor(rt, cbor::deserialize_params(params)?)?;
                Ok(RawBytes::default())
            }
            Some(Method::AddVerifier) => {
                Self::add_verifier(rt, cbor::deserialize_params(params)?)?;
                Ok(RawBytes::default())
            }
            Some(Method::RemoveVerifier) => {
                Self::remove_verifier(rt, cbor::deserialize_params(params)?)?;
                Ok(RawBytes::default())
            }
            Some(Method::AddVerifiedClient) => {
                Self::add_verified_client(rt, cbor::deserialize_params(params)?)?;
                Ok(RawBytes::default())
            }
            Some(Method::UseBytes) => {
                Self::use_bytes(rt, cbor::deserialize_params(params)?)?;
                Ok(RawBytes::default())
            }
            Some(Method::RestoreBytes) => {
                Self::restore_bytes(rt, cbor::deserialize_params(params)?)?;
                Ok(RawBytes::default())
            }
            Some(Method::RemoveVerifiedClientDataCap) => {
                let res =
                    Self::remove_verified_client_data_cap(rt, cbor::deserialize_params(params)?)?;
                Ok(RawBytes::serialize(res)?)
            }
            Some(Method::RemoveExpiredAllocations) => {
                let res = Self::remove_expired_allocations(rt, cbor::deserialize_params(params)?)?;
                Ok(RawBytes::serialize(res)?)
            }
            Some(Method::ClaimAllocations) => {
                let res = Self::claim_allocations(rt, cbor::deserialize_params(params)?)?;
                Ok(RawBytes::serialize(res)?)
            }
            Some(Method::UniversalReceiverHook) => {
                Self::universal_receiver_hook(rt, cbor::deserialize_params(params)?)?;
                Ok(RawBytes::default())
            }
            None => Err(actor_error!(unhandled_message; "Invalid method")),
        }
    }
}
