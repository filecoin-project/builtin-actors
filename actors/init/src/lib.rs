// Copyright 2019-2022 ChainSafe Systems
// SPDX-License-Identifier: Apache-2.0, MIT

use cid::Cid;
use fil_actors_runtime::runtime::builtins::Type;
use fil_actors_runtime::runtime::{ActorCode, Runtime};

use fil_actors_runtime::{
    actor_dispatch, actor_error, extract_send_result, ActorContext, ActorError, AsActorError,
    EAM_ACTOR_ADDR, SYSTEM_ACTOR_ADDR,
};
use fvm_shared::address::Address;
use fvm_shared::error::ExitCode;
use fvm_shared::{ActorID, METHOD_CONSTRUCTOR};
use num_derive::FromPrimitive;

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
        let (id_address, existing): (ActorID, bool) = rt.transaction(|s: &mut State, rt| {
            s.map_addresses_to_id(rt.store(), &robust_address, None)
                .context("failed to allocate ID address")
        })?;

        if existing {
            // NOTE: this case should be impossible, but we check it anyways just in case something
            // changes.
            return Err(actor_error!(
                forbidden,
                "cannot exec over an existing actor {}",
                id_address
            ));
        }

        // Create an empty actor
        rt.create_actor(params.code_cid, id_address, None)?;

        // Invoke constructor
        extract_send_result(rt.send_simple(
            &Address::new_id(id_address),
            METHOD_CONSTRUCTOR,
            params.constructor_params.into(),
            rt.message().value_received(),
        ))
        .context("constructor failed")?;

        Ok(ExecReturn { id_address: Address::new_id(id_address), robust_address })
    }

    /// Exec4 init actor
    pub fn exec4(rt: &mut impl Runtime, params: Exec4Params) -> Result<Exec4Return, ActorError> {
        if cfg!(feature = "m2-native") {
            rt.validate_immediate_caller_accept_any()?;
        } else {
            rt.validate_immediate_caller_is(std::iter::once(&EAM_ACTOR_ADDR))?;
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
        let (id_address, existing): (ActorID, bool) = rt.transaction(|s: &mut State, rt| {
            s.map_addresses_to_id(rt.store(), &robust_address, Some(&delegated_address))
                .context("failed to map addresses to ID")
        })?;

        // If the f4 address was already assigned, make sure we're deploying over a placeholder and not
        // some other existing actor (and make sure the target actor wasn't deleted either).
        if existing {
            let code_cid = rt
                .get_actor_code_cid(&id_address)
                .context_code(ExitCode::USR_FORBIDDEN, "cannot redeploy a deleted actor")?;
            let placeholder_cid = rt.get_code_cid_for_type(Type::Placeholder);
            if code_cid != placeholder_cid {
                return Err(ActorError::forbidden(format!(
                    "cannot replace an existing non-placeholder actor with code: {code_cid}"
                )));
            }
        }

        // Create an empty actor
        rt.create_actor(params.code_cid, id_address, Some(delegated_address))?;

        // Invoke constructor
        extract_send_result(rt.send_simple(
            &Address::new_id(id_address),
            METHOD_CONSTRUCTOR,
            params.constructor_params.into(),
            rt.message().value_received(),
        ))
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
    type Methods = Method;
    actor_dispatch! {
        Constructor => constructor,
        Exec => exec,
        Exec4 => exec4,
        #[cfg(feature = "m2-native")]
        InstallCode => install,
    }
}

#[cfg(not(feature = "m2-native"))]
fn can_exec(rt: &impl Runtime, caller: &Cid, exec: &Cid) -> bool {
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
