// Copyright 2024 Curio Storage Inc.
// SPDX-License-Identifier: Apache-2.0, MIT

use fvm_ipld_encoding::ipld_block::IpldBlock;
use fvm_shared::{METHOD_CONSTRUCTOR, MethodNum};
use num_derive::FromPrimitive;

use fil_actors_runtime::builtin::singletons::SYSTEM_ACTOR_ADDR;
use fil_actors_runtime::runtime::{ActorCode, Runtime};
use fil_actors_runtime::{FIRST_EXPORTED_METHOD_NUMBER, actor_dispatch};
use fil_actors_runtime::{ActorError, actor_error};

use crate::types::{ConstructorParams, SealerIDReturn};

pub use self::state::State;

mod state;
pub mod types;
pub mod testing;

#[cfg(feature = "fil-actor")]
fil_actors_runtime::wasm_trampoline!(Actor);

/// Sealer actor methods available
#[derive(FromPrimitive)]
#[repr(u64)]
pub enum Method {
    Constructor = METHOD_CONSTRUCTOR,
    SealerID = 2,
    // TODO: Add more methods as needed
}

/// Sealer Actor
pub struct Actor;

impl Actor {
    /// Constructor for Sealer actor
    pub fn constructor(rt: &impl Runtime, _params: ConstructorParams) -> Result<(), ActorError> {
        rt.validate_immediate_caller_is(std::iter::once(&SYSTEM_ACTOR_ADDR))?;
        let id_addr = rt.message().receiver();
        let state = State {
            id_addr,
            // TODO: initialize sector bitfield, acl, etc.
        };
        rt.create(&state)?;
        Ok(())
    }

    /// Returns the SealerID (the actor's ID address)
    pub fn sealer_id(rt: &impl Runtime) -> Result<SealerIDReturn, ActorError> {
        rt.validate_immediate_caller_accept_any()?;
        let st: State = rt.state()?;
        Ok(SealerIDReturn { id_addr: st.id_addr })
    }

    /// Fallback method for unimplemented method numbers.
    pub fn fallback(
        rt: &impl Runtime,
        method: MethodNum,
        _: Option<IpldBlock>,
    ) -> Result<Option<IpldBlock>, ActorError> {
        rt.validate_immediate_caller_accept_any()?;
        if method >= FIRST_EXPORTED_METHOD_NUMBER {
            Ok(None)
        } else {
            Err(actor_error!(unhandled_message; "invalid method: {}", method))
        }
    }
}

impl ActorCode for Actor {
    type Methods = Method;

    fn name() -> &'static str {
        "Sealer"
    }

    actor_dispatch! {
        Constructor => constructor,
        SealerID => sealer_id,
        _ => fallback,
    }
}
