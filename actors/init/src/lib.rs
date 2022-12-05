// Copyright 2019-2022 ChainSafe Systems
// SPDX-License-Identifier: Apache-2.0, MIT

use std::iter;

use cid::Cid;
use fil_actors_runtime::runtime::{ActorCode, Runtime};
use fil_actors_runtime::{
    actor_error, cbor, restrict_internal_api, ActorContext, ActorError, EAM_ACTOR_ADDR,
    SYSTEM_ACTOR_ADDR,
};
use fvm_ipld_encoding::RawBytes;
use fvm_shared::address::Address;
use fvm_shared::{ActorID, MethodNum, METHOD_CONSTRUCTOR};
use num_derive::FromPrimitive;
use num_traits::FromPrimitive;

pub use self::state::State;
pub use self::types::*;

mod state;
pub mod testing;
mod types;

#[cfg(feature = "fil-actor")]
fil_actors_runtime::wasm_trampoline!(Actor);

/// Init actor methods available
#[derive(FromPrimitive)]
#[repr(u64)]
pub enum Method {
    Constructor = METHOD_CONSTRUCTOR,
    Exec = 2,
    Exec4 = 3,
    #[cfg(feature = "m2-native")]
    InstallCode = 4,
    // Method numbers derived from FRC-0042 standards
    ExecExported = frc42_dispatch::method_hash!("Exec"),
    // TODO: Export Exec4
}

/// Init actor
pub struct Actor;

impl Actor {
    /// Init actor constructor
    pub fn constructor(rt: &mut impl Runtime, params: ConstructorParams) -> Result<(), ActorError> {
        let sys_ref: &Address = &SYSTEM_ACTOR_ADDR;
        rt.validate_immediate_caller_is(std::iter::once(sys_ref))?;
        let state = State::new(rt.store(), params.network_name)?;
        rt.create(&state)?;

        Ok(())
    }

    /// Exec init actor
    pub fn exec(rt: &mut impl Runtime, params: ExecParams) -> Result<ExecReturn, ActorError> {
        rt.validate_immediate_caller_accept_any()?;

        log::trace!("called exec; params.code_cid: {:?}", &params.code_cid);

        let caller_code =
            rt.get_actor_code_cid(&rt.message().caller().id().unwrap()).ok_or_else(|| {
                actor_error!(illegal_state, "no code for caller as {}", rt.message().caller())
            })?;

        log::trace!("caller code CID: {:?}", &caller_code);

        if !can_exec(rt, &caller_code, &params.code_cid) {
            return Err(actor_error!(forbidden;
                    "called type {} cannot exec actor type {}",
                    &caller_code, &params.code_cid
            ));
        }

        // Compute a re-org-stable address.
        // This address exists for use by messages coming from outside the system, in order to
        // stably address the newly created actor even if a chain re-org causes it to end up with
        // a different ID.
        let robust_address = rt.new_actor_address()?;

        log::trace!("robust address: {:?}", &robust_address);

        // Allocate an ID for this actor.
        // Store mapping of actor addresses to the actor ID.
        let id_address: ActorID = rt.transaction(|s: &mut State, rt| {
            s.map_address_to_new_id(rt.store(), &robust_address)
                .context("failed to allocate ID address")
        })?;

        // Create an empty actor
        rt.create_actor(params.code_cid, id_address, None)?;

        // Invoke constructor
        rt.send(
            &Address::new_id(id_address),
            METHOD_CONSTRUCTOR,
            params.constructor_params,
            rt.message().value_received(),
        )
        .context("constructor failed")?;

        Ok(ExecReturn { id_address: Address::new_id(id_address), robust_address })
    }

    /// Exec init actor
    pub fn exec4(rt: &mut impl Runtime, params: Exec4Params) -> Result<Exec4Return, ActorError> {
        if cfg!(feature = "m2-native") {
            rt.validate_immediate_caller_accept_any()?;
        } else {
            rt.validate_immediate_caller_is(iter::once(&EAM_ACTOR_ADDR))?;
        }

        // Compute the f4 address.
        let caller_id = rt.message().caller().id().unwrap();
        let delegated_address =
            Address::new_delegated(caller_id, &params.subaddress).map_err(|e| {
                ActorError::illegal_argument(format!("invalid delegated address: {}", e))
            })?;

        log::trace!("delegated address: {:?}", &delegated_address);

        // Compute a re-org-stable address.
        // This address exists for use by messages coming from outside the system, in order to
        // stably address the newly created actor even if a chain re-org causes it to end up with
        // a different ID.
        let robust_address = rt.new_actor_address()?;

        log::trace!("robust address: {:?}", &robust_address);

        // Allocate an ID for this actor.
        // Store mapping of actor addresses to the actor ID.
        let id_address: ActorID = rt.transaction(|s: &mut State, rt| {
            s.map_address_to_f4(rt.store(), &robust_address, &delegated_address)
                .context("constructor failed")
        })?;

        // Create an empty actor
        rt.create_actor(params.code_cid, id_address, Some(delegated_address))?;

        // Invoke constructor
        rt.send(
            &Address::new_id(id_address),
            METHOD_CONSTRUCTOR,
            params.constructor_params,
            rt.message().value_received(),
        )
        .context("constructor failed")?;

        Ok(Exec4Return { id_address: Address::new_id(id_address), robust_address })
    }

    #[cfg(feature = "m2-native")]
    pub fn install(
        rt: &mut impl Runtime,
        params: InstallParams,
    ) -> Result<InstallReturn, ActorError> {
        use cid::multihash::Code;
        use fil_actors_runtime::AsActorError;
        use fvm_ipld_blockstore::{Block, Blockstore};
        use fvm_shared::error::ExitCode;

        rt.validate_immediate_caller_accept_any()?;

        let (code_cid, installed) = rt.transaction(|st: &mut State, rt| {
            let code = params.code.bytes();
            let code_cid = rt.store().put(Code::Blake2b256, &Block::new(0x55, code)).context_code(
                ExitCode::USR_SERIALIZATION,
                "failed to put code into the bockstore",
            )?;

            if st.is_installed_actor(rt.store(), &code_cid).context_code(
                ExitCode::USR_ILLEGAL_STATE,
                "failed to check state for installed actor",
            )? {
                return Ok((code_cid, false));
            }

            rt.install_actor(&code_cid).context_code(
                ExitCode::USR_ILLEGAL_ARGUMENT,
                "failed to check state for installed actor",
            )?;

            st.add_installed_actor(rt.store(), code_cid).context_code(
                ExitCode::USR_ILLEGAL_STATE,
                "failed to add installed actor to state",
            )?;
            Ok((code_cid, true))
        })?;

        Ok(InstallReturn { code_cid, installed })
    }
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
                Self::constructor(rt, cbor::deserialize_params(params)?)?;
                Ok(RawBytes::default())
            }
            Some(Method::Exec) | Some(Method::ExecExported) => {
                let res = Self::exec(rt, cbor::deserialize_params(params)?)?;
                Ok(RawBytes::serialize(res)?)
            }
            Some(Method::Exec4) => {
                let res = Self::exec4(rt, cbor::deserialize_params(params)?)?;
                Ok(RawBytes::serialize(res)?)
            }
            #[cfg(feature = "m2-native")]
            Some(Method::InstallCode) => {
                let res = Self::install(rt, cbor::deserialize_params(params)?)?;
                Ok(RawBytes::serialize(res)?)
            }
            None => Err(actor_error!(unhandled_message; "Invalid method")),
        }
    }
}

#[cfg(not(feature = "m2-native"))]
fn can_exec(rt: &impl Runtime, caller: &Cid, exec: &Cid) -> bool {
    use fil_actors_runtime::runtime::builtins::Type;
    rt.resolve_builtin_actor_type(exec)
        .map(|typ| match typ {
            Type::Multisig | Type::PaymentChannel => true,
            Type::Miner if rt.resolve_builtin_actor_type(caller) == Some(Type::Power) => true,
            _ => false,
        })
        .unwrap_or(false)
}

#[cfg(feature = "m2-native")]
fn can_exec(_rt: &impl Runtime, _caller: &Cid, _exec: &Cid) -> bool {
    // TODO figure out ACLs -- m2-native allows exec for everyone for now
    //      maybe we should leave this as is for production, but at least we should
    //      consider adding relevant ACLs.
    true
}
