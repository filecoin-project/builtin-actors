// Copyright 2019-2022 ChainSafe Systems
// SPDX-License-Identifier: Apache-2.0, MIT

use fil_actors_runtime::runtime::{ActorCode, Runtime};
use fil_actors_runtime::{
    actor_error, cbor, ActorDowncast, ActorError, SYSTEM_ACTOR_ADDR,
};
use fvm_ipld_blockstore::Blockstore;
use fvm_shared::error::ExitCode;
use fvm_shared::{MethodNum,  METHOD_CONSTRUCTOR};
use num_derive::FromPrimitive;
use num_traits::{FromPrimitive};
use fvm_ipld_encoding::RawBytes;

pub use self::state::*;
pub use self::types::*;

#[cfg(feature = "fil-actor")]
fil_actors_runtime::wasm_trampoline!(Actor);

#[doc(hidden)]
pub mod ext;
mod state;
mod types;

/// Storage power actor methods available
#[derive(FromPrimitive)]
#[repr(u64)]
pub enum Method {
    /// Constructor for Storage Power Actor
    Constructor = METHOD_CONSTRUCTOR,
}

/// Subnet Coordinator Actor
pub struct Actor;
impl Actor {
    /// Constructor for SCA actor
    fn constructor<BS, RT>(rt: &mut RT, params: ConstructorParams) -> Result<(), ActorError>
    where
        BS: Blockstore,
        RT: Runtime<BS>,
    {
        rt.validate_immediate_caller_is(std::iter::once(&*SYSTEM_ACTOR_ADDR))?;

        let st = State::new(rt.store(), params).map_err(|e| {
            e.downcast_default(ExitCode::ErrIllegalState, "Failed to create SCA actor state")
        })?;
        rt.create(&st)?;
        Ok(())
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
            None => Err(actor_error!(SysErrInvalidMethod; "Invalid method")),
        }
    }
}
