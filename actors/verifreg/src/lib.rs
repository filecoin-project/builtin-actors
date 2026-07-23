// Copyright 2019-2022 ChainSafe Systems
// SPDX-License-Identifier: Apache-2.0, MIT

use frc46_token::token::TOKEN_PRECISION;
use frc46_token::token::types::TransferParams;
use fvm_actor_utils::receiver::UniversalReceiverParams;
use fvm_ipld_encoding::ipld_block::IpldBlock;
use fvm_shared::address::Address;
use fvm_shared::econ::TokenAmount;
use fvm_shared::error::ExitCode;
use fvm_shared::{ActorID, METHOD_CONSTRUCTOR};
use log::info;
use num_derive::FromPrimitive;
use num_traits::Zero;

use fil_actors_runtime::runtime::builtins::Type;
use fil_actors_runtime::runtime::{ActorCode, Runtime};
use fil_actors_runtime::{ActorContext, AsActorError, BatchReturnGen};
use fil_actors_runtime::{
    ActorError, BatchReturn, DATACAP_TOKEN_ACTOR_ADDR, SYSTEM_ACTOR_ADDR, actor_dispatch,
    actor_error, extract_send_result,
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

// FIP-0118: datacap is deprecated; callers still apply their own caller validation first.
fn datacap_deprecated<T>(reason: &str) -> Result<T, ActorError> {
    Err(actor_error!(
        forbidden,
        "FIP-0118: datacap is deprecated, {} is no longer supported",
        reason
    ))
}

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
        // FIP-0118: datacap minting is deprecated. No new verifiers can be added.
        datacap_deprecated("adding new verifiers")
    }

    pub fn remove_verifier(
        rt: &impl Runtime,
        _params: RemoveVerifierParams,
    ) -> Result<(), ActorError> {
        rt.validate_immediate_caller_accept_any()?;
        // FIP-0118: datacap is deprecated. Verifiers are no longer managed on-chain.
        datacap_deprecated("removing verifiers")
    }

    pub fn add_verified_client(
        rt: &impl Runtime,
        _params: AddVerifiedClientParams,
    ) -> Result<(), ActorError> {
        rt.validate_immediate_caller_accept_any()?;
        // FIP-0118: datacap minting is deprecated. No new datacap can be granted.
        datacap_deprecated("minting new datacap")
    }

    pub fn remove_verified_client_data_cap(
        rt: &impl Runtime,
        _params: RemoveDataCapParams,
    ) -> Result<RemoveDataCapReturn, ActorError> {
        rt.validate_immediate_caller_accept_any()?;
        // FIP-0118: datacap is deprecated. Verified clients' datacap balances are frozen in place.
        datacap_deprecated("removing verified client data cap")
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
    // FIP-0118: claim allocations is disabled. The miner actor no longer calls this method.
    pub fn claim_allocations(
        rt: &impl Runtime,
        _params: ClaimAllocationsParams,
    ) -> Result<ClaimAllocationsReturn, ActorError> {
        rt.validate_immediate_caller_type(std::iter::once(&Type::Miner))?;
        datacap_deprecated("claim allocations")
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
    // FIP-0118: extend claim terms is disabled. Claim extensions are pointless
    // without QAP benefit.
    pub fn extend_claim_terms(
        rt: &impl Runtime,
        _params: ExtendClaimTermsParams,
    ) -> Result<ExtendClaimTermsReturn, ActorError> {
        rt.validate_immediate_caller_accept_any()?;
        datacap_deprecated("extending claim terms")
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
    // FIP-0118: datacap is deprecated. No new allocations or claim extensions can be created.
    // This method now always rejects incoming datacap tokens.
    pub fn universal_receiver_hook(
        rt: &impl Runtime,
        _params: UniversalReceiverParams,
    ) -> Result<AllocationsResponse, ActorError> {
        rt.validate_immediate_caller_is(&[DATACAP_TOKEN_ACTOR_ADDR])?;
        datacap_deprecated("new allocations")
    }
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
