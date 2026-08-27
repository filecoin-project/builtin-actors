// Copyright 2019-2022 ChainSafe Systems
// SPDX-License-Identifier: Apache-2.0, MIT

use fvm_actor_utils::receiver::UniversalReceiverParams;
use fvm_shared::METHOD_CONSTRUCTOR;
use fvm_shared::address::Address;
use fvm_shared::error::ExitCode;
use log::info;
use num_derive::FromPrimitive;

use fil_actors_runtime::runtime::builtins::Type;
use fil_actors_runtime::runtime::{ActorCode, Runtime};
use fil_actors_runtime::{ActorContext, AsActorError, BatchReturnGen};
use fil_actors_runtime::{
    ActorError, DATACAP_TOKEN_ACTOR_ADDR, SYSTEM_ACTOR_ADDR, actor_dispatch, actor_error,
};

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

// Callers apply their own caller validation before calling this.
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
        datacap_deprecated("adding new verifiers")
    }

    pub fn remove_verifier(
        rt: &impl Runtime,
        _params: RemoveVerifierParams,
    ) -> Result<(), ActorError> {
        rt.validate_immediate_caller_accept_any()?;
        datacap_deprecated("removing verifiers")
    }

    pub fn add_verified_client(
        rt: &impl Runtime,
        _params: AddVerifiedClientParams,
    ) -> Result<(), ActorError> {
        rt.validate_immediate_caller_accept_any()?;
        datacap_deprecated("minting new datacap")
    }

    pub fn remove_verified_client_data_cap(
        rt: &impl Runtime,
        _params: RemoveDataCapParams,
    ) -> Result<RemoveDataCapReturn, ActorError> {
        rt.validate_immediate_caller_accept_any()?;
        datacap_deprecated("removing verified client data cap")
    }

    pub fn remove_expired_allocations(
        rt: &impl Runtime,
        _params: RemoveExpiredAllocationsParams,
    ) -> Result<RemoveExpiredAllocationsReturn, ActorError> {
        rt.validate_immediate_caller_accept_any()?;
        // FIP-0118: the network upgrade migration clears all pending allocations, so
        // there is nothing left to ever expire.
        datacap_deprecated("removing expired allocations")
    }

    pub fn claim_allocations(
        rt: &impl Runtime,
        _params: ClaimAllocationsParams,
    ) -> Result<ClaimAllocationsReturn, ActorError> {
        rt.validate_immediate_caller_type(std::iter::once(&Type::Miner))?;
        datacap_deprecated("claiming allocations")
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

    /// Always rejects. Extending a claim's term bought more time at 10x quality, which
    /// FIP-0118 grants unconditionally, so there is nothing left to extend.
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
        _params: RemoveExpiredClaimsParams,
    ) -> Result<RemoveExpiredClaimsReturn, ActorError> {
        rt.validate_immediate_caller_accept_any()?;
        datacap_deprecated("removing expired claims")
    }

    // Always rejects. This received datacap tokens and created allocations from them;
    // FIP-0118 disables allocations, and datacap balances are frozen so none can arrive.
    pub fn universal_receiver_hook(
        rt: &impl Runtime,
        _params: UniversalReceiverParams,
    ) -> Result<AllocationsResponse, ActorError> {
        rt.validate_immediate_caller_is(&[DATACAP_TOKEN_ACTOR_ADDR])?;
        datacap_deprecated("new allocations")
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
