// Copyright 2019-2022 ChainSafe Systems
// SPDX-License-Identifier: Apache-2.0, MIT

use cid::Cid;
use fil_actors_runtime::runtime::{ActorCode, Runtime};
use fil_actors_runtime::{actor_error, cbor, ActorContext2, ActorError, SYSTEM_ACTOR_ADDR};
use fvm_ipld_blockstore::Blockstore;
use fvm_ipld_encoding::RawBytes;
use fvm_shared::actor::builtin::Type;
use fvm_shared::address::Address;
use fvm_shared::error::ExitCode;
use fvm_shared::{ActorID, MethodNum, METHOD_CONSTRUCTOR};
use num_derive::FromPrimitive;
use num_traits::FromPrimitive;

pub use self::state::State;
pub use self::types::*;

mod state;
mod types;

#[cfg(feature = "fil-actor")]
fil_actors_runtime::wasm_trampoline!(Actor);

// * Updated to specs-actors commit: 999e57a151cc7ada020ca2844b651499ab8c0dec (v3.0.1)

/// Init actor methods available
#[derive(FromPrimitive)]
#[repr(u64)]
pub enum Method {
    Constructor = METHOD_CONSTRUCTOR,
    Exec = 2,
}

/// Init actor
pub struct Actor;
impl Actor {
    /// Init actor constructor
    pub fn constructor<BS, RT>(rt: &mut RT, params: ConstructorParams) -> Result<(), ActorError>
    where
        BS: Blockstore,
        RT: Runtime<BS>,
    {
        let sys_ref: &Address = &SYSTEM_ACTOR_ADDR;
        rt.validate_immediate_caller_is(std::iter::once(sys_ref))?;
        let state = State::new(rt.store(), params.network_name)?;

        rt.create(&state)?;

        Ok(())
    }

    /// Exec init actor
    pub fn exec<BS, RT>(rt: &mut RT, params: ExecParams) -> Result<ExecReturn, ActorError>
    where
        BS: Blockstore,
        RT: Runtime<BS>,
    {
        rt.validate_immediate_caller_accept_any()?;

        log::trace!("called exec; params.code_cid: {:?}", &params.code_cid);

        let caller_code = rt.get_actor_code_cid(&rt.message().caller()).ok_or_else(|| {
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
        // Store mapping of pubkey or actor address to actor ID
        let id_address: ActorID = rt.transaction(|s: &mut State, rt| {
            s.map_address_to_new_id(rt.store(), &robust_address)
                .context_code(ExitCode::USR_ILLEGAL_STATE, "failed to allocate ID address")
        })?;

        // Create an empty actor
        rt.create_actor(params.code_cid, id_address)?;

        // Invoke constructor
        rt.send(
            Address::new_id(id_address),
            METHOD_CONSTRUCTOR,
            params.constructor_params,
            rt.message().value_received(),
        )
        .map_err(|err| err.wrap("constructor failed"))?;

        Ok(ExecReturn { id_address: Address::new_id(id_address), robust_address })
    }
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
            Some(Method::Exec) => {
                let res = Self::exec(rt, cbor::deserialize_params(params)?)?;
                Ok(RawBytes::serialize(res).exit_code(ExitCode::USR_ILLEGAL_STATE)?)
            }
            None => Err(actor_error!(unhandled_message; "Invalid method")),
        }
    }
}

fn can_exec<BS, RT>(rt: &RT, caller: &Cid, exec: &Cid) -> bool
where
    BS: Blockstore,
    RT: Runtime<BS>,
{
    rt.resolve_builtin_actor_type(exec)
        .map(|typ| match typ {
            Type::Multisig | Type::PaymentChannel => true,
            Type::Miner if rt.resolve_builtin_actor_type(caller) == Some(Type::Power) => true,
            _ => false,
        })
        .unwrap_or(false)
}
