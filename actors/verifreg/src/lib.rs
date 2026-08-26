// Copyright 2019-2022 ChainSafe Systems
// SPDX-License-Identifier: Apache-2.0, MIT

use frc46_token::token::TOKEN_PRECISION;
use frc46_token::token::types::TransferParams;
use fvm_actor_utils::receiver::UniversalReceiverParams;
use fvm_ipld_blockstore::Blockstore;
use fvm_ipld_encoding::RawBytes;
use fvm_ipld_encoding::ipld_block::IpldBlock;
use fvm_shared::address::Address;
use fvm_shared::bigint::BigInt;
use fvm_shared::econ::TokenAmount;
use fvm_shared::error::ExitCode;
use fvm_shared::sys::SendFlags;
use fvm_shared::{ActorID, METHOD_CONSTRUCTOR};
use log::info;
use num_derive::FromPrimitive;
use num_traits::Zero;

use fil_actors_runtime::runtime::builtins::Type;
use fil_actors_runtime::runtime::{ActorCode, Runtime};
use fil_actors_runtime::{ActorContext, AsActorError, BatchReturnGen};
use fil_actors_runtime::{
    ActorError, BatchReturn, DATACAP_TOKEN_ACTOR_ADDR, SYSTEM_ACTOR_ADDR,
    VERIFIED_REGISTRY_ACTOR_ADDR, actor_dispatch, actor_error, deserialize_block,
    extract_send_result, resolve_to_actor_id,
};

use crate::ext::datacap::DestroyParams;
use crate::state::{
    DATACAP_MAP_CONFIG, DataCapMap, REMOVE_DATACAP_PROPOSALS_CONFIG, RemoveDataCapProposalMap,
};

pub use self::state::Allocation;
pub use self::state::Claim;
pub use self::state::State;
pub use self::types::*;

#[cfg(feature = "fil-actor")]
fil_actors_runtime::wasm_trampoline!(Actor);

mod emit;

pub mod expiration;
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
    // UseBytes = 5,     // Deprecated
    // RestoreBytes = 6, // Deprecated
    RemoveVerifiedClientDataCap = 7,
    RemoveExpiredAllocations = 8,
    ClaimAllocations = 9,
    GetClaims = 10,
    ExtendClaimTerms = 11,
    RemoveExpiredClaims = 12,
    // Method numbers derived from FRC-0042 standards
    AddVerifiedClientExported = frc42_dispatch::method_hash!("AddVerifiedClient"),
    RemoveExpiredAllocationsExported = frc42_dispatch::method_hash!("RemoveExpiredAllocations"),
    GetClaimsExported = frc42_dispatch::method_hash!("GetClaims"),
    ExtendClaimTermsExported = frc42_dispatch::method_hash!("ExtendClaimTerms"),
    RemoveExpiredClaimsExported = frc42_dispatch::method_hash!("RemoveExpiredClaims"),
    UniversalReceiverHook = frc42_dispatch::method_hash!("Receive"),
}

pub struct Actor;

impl Actor {
    /// Constructor for Registry Actor
    pub fn constructor(rt: &impl Runtime, params: ConstructorParams) -> Result<(), ActorError> {
        rt.validate_immediate_caller_is(std::iter::once(&SYSTEM_ACTOR_ADDR))?;

        // root should be an ID address
        let id_addr = rt
            .resolve_address(&params.root_key)
            .context_code(ExitCode::USR_ILLEGAL_ARGUMENT, "root should be an ID address")?;

        let st = State::new(rt.store(), Address::new_id(id_addr))
            .context("failed to create verifreg state")?;

        rt.create(&st)?;
        Ok(())
    }

    pub fn add_verifier(rt: &impl Runtime, _params: AddVerifierParams) -> Result<(), ActorError> {
        rt.validate_immediate_caller_accept_any()?;
        // FIP-1249: datacap minting is deprecated. No new verifiers can be added.
        Err(actor_error!(
            forbidden,
            "FIP-1249: datacap is deprecated, adding new verifiers is no longer supported"
        ))
    }

    pub fn remove_verifier(
        rt: &impl Runtime,
        params: RemoveVerifierParams,
    ) -> Result<(), ActorError> {
        let verifier = resolve_to_actor_id(rt, &params.verifier, false)?;
        let verifier_addr = Address::new_id(verifier);

        rt.transaction(|st: &mut State, rt| {
            rt.validate_immediate_caller_is(std::iter::once(&st.root_key))?;
            st.remove_verifier(rt.store(), &verifier_addr).context("failed to remove verifier")
        })?;

        emit::verifier_balance(rt, verifier, &DataCap::zero(), None)
    }

    pub fn add_verified_client(
        rt: &impl Runtime,
        _params: AddVerifiedClientParams,
    ) -> Result<(), ActorError> {
        rt.validate_immediate_caller_accept_any()?;
        // FIP-1249: datacap minting is deprecated. No new datacap can be granted.
        Err(actor_error!(
            forbidden,
            "FIP-1249: datacap is deprecated, minting new datacap is no longer supported"
        ))
    }

    /// Removes DataCap allocated to a verified client.
    pub fn remove_verified_client_data_cap(
        rt: &impl Runtime,
        params: RemoveDataCapParams,
    ) -> Result<RemoveDataCapReturn, ActorError> {
        let client = resolve_to_actor_id(rt, &params.verified_client_to_remove, false)?;
        let client = Address::new_id(client);

        let verifier_1 = resolve_to_actor_id(rt, &params.verifier_request_1.verifier, true)?;
        let verifier_1 = Address::new_id(verifier_1);

        let verifier_2 = resolve_to_actor_id(rt, &params.verifier_request_2.verifier, true)?;
        let verifier_2 = Address::new_id(verifier_2);

        if verifier_1 == verifier_2 {
            return Err(actor_error!(
                illegal_argument,
                "need two different verifiers to send remove datacap request"
            ));
        }

        let (verifier_1_id, verifier_2_id) = rt.transaction(|st: &mut State, rt| {
            rt.validate_immediate_caller_is(std::iter::once(&st.root_key))?;

            if params.verified_client_to_remove == VERIFIED_REGISTRY_ACTOR_ADDR {
                return Err(actor_error!(
                    illegal_argument,
                    "cannot remove data cap from verified registry itself"
                ));
            }

            if !is_verifier(rt, st, verifier_1)? {
                return Err(actor_error!(not_found, "{} is not a verifier", verifier_1));
            }

            if !is_verifier(rt, st, verifier_2)? {
                return Err(actor_error!(not_found, "{} is not a verifier", verifier_2));
            }

            // validate signatures
            let mut proposal_ids = RemoveDataCapProposalMap::load(
                rt.store(),
                &st.remove_data_cap_proposal_ids,
                REMOVE_DATACAP_PROPOSALS_CONFIG,
                "remove datacap proposals",
            )?;

            let verifier_1_id = use_proposal_id(&mut proposal_ids, verifier_1, client)?;
            let verifier_2_id = use_proposal_id(&mut proposal_ids, verifier_2, client)?;

            // Assume proposal ids are valid and increment them
            st.remove_data_cap_proposal_ids = proposal_ids.flush()?;
            Ok((verifier_1_id, verifier_2_id))
        })?;

        // Now make sure the proposals were actually valid. We had to increment them first in case
        // re-entrant calls do anything funny.
        //
        // If this fails, we'll revert and the proposals will be restored.
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

        // Burn the client's data cap tokens.
        let balance = balance(rt, &client).context("failed to fetch balance")?;
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
    // If no allocations are specified, all eligible allocations are removed.
    pub fn remove_expired_allocations(
        rt: &impl Runtime,
        params: RemoveExpiredAllocationsParams,
    ) -> Result<RemoveExpiredAllocationsReturn, ActorError> {
        // Since the allocations are expired, this is safe to be called by anyone.
        rt.validate_immediate_caller_accept_any()?;
        let curr_epoch = rt.curr_epoch();
        let mut batch_ret = BatchReturn::empty();
        let mut considered = Vec::<ClaimID>::new();
        let mut recovered_datacap = DataCap::zero();
        let recovered_datacap = rt
            .transaction(|st: &mut State, rt| {
                let mut allocs = st.load_allocs(rt.store())?;

                let to_remove: Vec<&AllocationID>;
                if params.allocation_ids.is_empty() {
                    // Find all expired allocations for the client.
                    considered = expiration::find_expired(&mut allocs, params.client, curr_epoch)?;
                    batch_ret = BatchReturn::ok(considered.len() as u32);
                    to_remove = considered.iter().collect();
                } else {
                    considered = params.allocation_ids.clone();
                    batch_ret = expiration::check_expired(
                        &mut allocs,
                        &params.allocation_ids,
                        params.client,
                        curr_epoch,
                    )?;
                    to_remove = batch_ret.successes(&params.allocation_ids);
                }

                for id in to_remove {
                    let existing = allocs
                        .remove(params.client, *id)
                        .context_code(
                            ExitCode::USR_ILLEGAL_STATE,
                            format!("failed to remove allocation {}", id),
                        )?
                        .unwrap(); // Unwrapping here as both paths to here should ensure the allocation exists.

                    emit::allocation_removed(rt, *id, &existing)?;

                    // Unwrapping here as both paths to here should ensure the allocation exists.
                    recovered_datacap += existing.size.0;
                }

                st.save_allocs(&mut allocs)?;
                Ok(recovered_datacap)
            })
            .context("state transaction failed")?;

        // Transfer the recovered datacap back to the client.
        transfer(rt, params.client, &recovered_datacap).with_context(|| {
            format!(
                "failed to transfer recovered datacap {} back to client {}",
                &recovered_datacap, params.client
            )
        })?;

        Ok(RemoveExpiredAllocationsReturn {
            considered,
            results: batch_ret,
            datacap_recovered: recovered_datacap,
        })
    }

    /// Called by storage provider actor to claim allocations for data provably committed to storage.
    /// For each allocation claim, the registry checks that the provided piece CID
    /// and size match that of the allocation.
    /// Claims are processed in groups by sector. A failed claim will cause the
    /// others in its group to fail too, unless `all_or_nothing` is enabled, in which case
    /// the method will abort.
    /// Returns an indicator of success for each sector group, and the size of claimed space.
    // FIP-1249: claim allocations is disabled. The miner actor no longer calls this method.
    pub fn claim_allocations(
        rt: &impl Runtime,
        _params: ClaimAllocationsParams,
    ) -> Result<ClaimAllocationsReturn, ActorError> {
        rt.validate_immediate_caller_type(std::iter::once(&Type::Miner))?;
        Err(actor_error!(
            forbidden,
            "FIP-1249: datacap is deprecated, claim allocations is no longer supported"
        ))
    }

    // get claims for a provider
    pub fn get_claims(
        rt: &impl Runtime,
        params: GetClaimsParams,
    ) -> Result<GetClaimsReturn, ActorError> {
        rt.validate_immediate_caller_accept_any()?;
        let mut batch_gen = BatchReturnGen::new(params.claim_ids.len());
        let st: State = rt.state()?;
        let mut st_claims = st.load_claims(rt.store())?;
        let mut claims = Vec::new();
        for id in params.claim_ids {
            let maybe_claim = state::get_claim(&mut st_claims, params.provider, id)?;
            match maybe_claim {
                None => {
                    batch_gen.add_fail(ExitCode::USR_NOT_FOUND);
                    info!("no claim {} for provider {}", id, params.provider,);
                }
                Some(claim) => {
                    batch_gen.add_success();
                    claims.push(claim.clone());
                }
            };
        }

        Ok(GetClaimsReturn { batch_info: batch_gen.generate(), claims })
    }

    /// Extends the maximum term of some claims up to the largest value they could have been
    /// originally allocated.
    /// Callable only by the claims' client.
    /// Cannot reduce a claim's term.
    /// Can extend the term even if the claim has already expired.
    /// Note that this method can't extend the term past the original limit,
    // FIP-1249: extend claim terms is disabled. Claim extensions are pointless
    // without QAP benefit.
    pub fn extend_claim_terms(
        rt: &impl Runtime,
        _params: ExtendClaimTermsParams,
    ) -> Result<ExtendClaimTermsReturn, ActorError> {
        rt.validate_immediate_caller_accept_any()?;
        Err(actor_error!(
            forbidden,
            "FIP-1249: datacap is deprecated, extending claim terms is no longer supported"
        ))
    }

    // A claim may be removed after its maximum term has elapsed (by anyone).
    // If no claims are specified, all eligible claims are removed.
    pub fn remove_expired_claims(
        rt: &impl Runtime,
        params: RemoveExpiredClaimsParams,
    ) -> Result<RemoveExpiredClaimsReturn, ActorError> {
        // Since the claims are expired, this is safe to be called by anyone.
        rt.validate_immediate_caller_accept_any()?;
        let curr_epoch = rt.curr_epoch();
        let mut batch_ret = BatchReturn::empty();
        let mut considered = Vec::<ClaimID>::new();
        rt.transaction(|st: &mut State, rt| {
            let mut claims = st.load_claims(rt.store())?;
            let to_remove: Vec<&ClaimID>;
            if params.claim_ids.is_empty() {
                // Find all expired claims for the provider.
                considered = expiration::find_expired(&mut claims, params.provider, curr_epoch)?;
                batch_ret = BatchReturn::ok(considered.len() as u32);
                to_remove = considered.iter().collect();
            } else {
                considered = params.claim_ids.clone();
                batch_ret = expiration::check_expired(
                    &mut claims,
                    &params.claim_ids,
                    params.provider,
                    curr_epoch,
                )?;
                to_remove = batch_ret.successes(&params.claim_ids);
            }

            for id in to_remove {
                let removed = claims
                    .remove(params.provider, *id)
                    .context_code(
                        ExitCode::USR_ILLEGAL_STATE,
                        format!("failed to remove claim {}", id),
                    )?
                    .unwrap();

                emit::claim_removed(rt, *id, &removed)?;
            }

            st.save_claims(&mut claims)?;
            Ok(())
        })
        .context("state transaction failed")?;

        Ok(RemoveExpiredClaimsReturn { considered, results: batch_ret })
    }

    // Receives data cap tokens (only) and creates allocations according to one or more
    // allocation requests specified in the transfer's operator data.
    // The token amount received must exactly correspond to the sum of the requested allocation sizes.
    // FIP-1249: datacap is deprecated. No new allocations or claim extensions can be created.
    // This method now always rejects incoming datacap tokens.
    pub fn universal_receiver_hook(
        rt: &impl Runtime,
        _params: UniversalReceiverParams,
    ) -> Result<AllocationsResponse, ActorError> {
        rt.validate_immediate_caller_is(&[DATACAP_TOKEN_ACTOR_ADDR])?;
        Err(actor_error!(
            forbidden,
            "FIP-1249: datacap is deprecated, new allocations are no longer supported"
        ))
    }
}

// Checks whether an address has a verifier entry (which could be zero).
fn is_verifier(rt: &impl Runtime, st: &State, address: Address) -> Result<bool, ActorError> {
    let verifiers = DataCapMap::load(rt.store(), &st.verifiers, DATACAP_MAP_CONFIG, "verifiers")?;
    // check that the `address` is currently a verified client
    let found = verifiers.contains_key(&address)?;
    Ok(found)
}

// Invokes Balance on the data cap token actor, and converts the result to whole units of data cap.
fn balance(rt: &impl Runtime, owner: &Address) -> Result<DataCap, ActorError> {
    let params = IpldBlock::serialize_cbor(owner)?;
    let x: TokenAmount = deserialize_block(
        extract_send_result(rt.send(
            &DATACAP_TOKEN_ACTOR_ADDR,
            ext::datacap::Method::Balance as u64,
            params,
            TokenAmount::zero(),
            None,
            SendFlags::READ_ONLY,
        ))
        .context(format!("failed to query datacap balance of {}", owner))?,
    )?;
    Ok(tokens_to_datacap(&x))
}

// Invokes Destroy on a data cap token actor for whole units of data cap.
fn destroy(rt: &impl Runtime, owner: &Address, amount: &DataCap) -> Result<(), ActorError> {
    if amount.is_zero() {
        return Ok(());
    }
    let token_amt = datacap_to_tokens(amount);
    let params = DestroyParams { owner: *owner, amount: token_amt };
    extract_send_result(rt.send_simple(
        &DATACAP_TOKEN_ACTOR_ADDR,
        ext::datacap::Method::Destroy as u64,
        IpldBlock::serialize_cbor(&params)?,
        TokenAmount::zero(),
    ))
    .context(format!("failed to send destroy {:?} to datacap", params))?;
    Ok(())
}

// Invokes transfer on a data cap token actor for whole units of data cap.
fn transfer(rt: &impl Runtime, to: ActorID, amount: &DataCap) -> Result<(), ActorError> {
    let token_amt = datacap_to_tokens(amount);
    let params = TransferParams {
        to: Address::new_id(to),
        amount: token_amt,
        operator_data: Default::default(),
    };
    extract_send_result(rt.send_simple(
        &DATACAP_TOKEN_ACTOR_ADDR,
        ext::datacap::Method::Transfer as u64,
        IpldBlock::serialize_cbor(&params)?,
        TokenAmount::zero(),
    ))
    .context(format!("failed to send transfer to datacap {:?}", params))?;
    Ok(())
}

fn datacap_to_tokens(amount: &DataCap) -> TokenAmount {
    TokenAmount::from_atto(amount.clone()) * TOKEN_PRECISION
}

fn tokens_to_datacap(amount: &TokenAmount) -> BigInt {
    amount.atto() / TOKEN_PRECISION
}

fn use_proposal_id<BS>(
    proposal_ids: &mut RemoveDataCapProposalMap<BS>,
    verifier: Address,
    client: Address,
) -> Result<RemoveDataCapProposalID, ActorError>
where
    BS: Blockstore,
{
    let key = AddrPairKey::new(verifier, client);
    let maybe_id =
        proposal_ids.get(&key).with_context(|| format!("verifier {verifier} client {client}"))?;

    let curr_id = if let Some(RemoveDataCapProposalID { id }) = maybe_id {
        RemoveDataCapProposalID { id: *id }
    } else {
        RemoveDataCapProposalID { id: 0 }
    };

    let next_id = RemoveDataCapProposalID { id: curr_id.id + 1 };
    proposal_ids
        .set(&key, next_id)
        .with_context(|| format!("verifier {verifier} client {client}"))?;
    Ok(curr_id)
}

fn remove_data_cap_request_is_valid(
    rt: &impl Runtime,
    request: &RemoveDataCapRequest,
    id: RemoveDataCapProposalID,
    to_remove: &DataCap,
    client: Address,
) -> Result<(), ActorError> {
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

    if !extract_send_result(rt.send(
        &request.verifier,
        ext::account::AUTHENTICATE_MESSAGE_METHOD,
        IpldBlock::serialize_cbor(&ext::account::AuthenticateMessageParams {
            signature: request.signature.bytes.clone(),
            message: payload,
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

impl ActorCode for Actor {
    type Methods = Method;

    fn name() -> &'static str {
        "VerifiedRegistry"
    }

    actor_dispatch! {
        Constructor => constructor,
        AddVerifier => add_verifier,
        RemoveVerifier => remove_verifier,
        AddVerifiedClient|AddVerifiedClientExported => add_verified_client,
        RemoveVerifiedClientDataCap => remove_verified_client_data_cap,
        RemoveExpiredAllocations|RemoveExpiredAllocationsExported => remove_expired_allocations,
        ClaimAllocations => claim_allocations,
        GetClaims|GetClaimsExported => get_claims,
        ExtendClaimTerms|ExtendClaimTermsExported => extend_claim_terms,
        RemoveExpiredClaims|RemoveExpiredClaimsExported => remove_expired_claims,
        UniversalReceiverHook => universal_receiver_hook,
    }
}
