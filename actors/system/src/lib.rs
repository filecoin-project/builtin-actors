// Copyright 2019-2022 ChainSafe Systems
// SPDX-License-Identifier: Apache-2.0, MIT
use cid::{multihash, Cid};
use fvm_ipld_blockstore::Blockstore;
use fvm_ipld_encoding::tuple::*;
use fvm_ipld_encoding::CborStore;
use fvm_shared::error::ExitCode;
use fvm_shared::METHOD_CONSTRUCTOR;
use num_derive::FromPrimitive;

use fil_actors_runtime::runtime::{ActorCode, Runtime};
use fil_actors_runtime::{
    actor_dispatch, actor_error, ActorContext, ActorError, AsActorError, SYSTEM_ACTOR_ADDR,
};

#[cfg(feature = "fil-actor")]
fil_actors_runtime::wasm_trampoline!(Actor);

/// System actor methods.
#[derive(FromPrimitive)]
#[repr(u64)]
pub enum Method {
    Constructor = METHOD_CONSTRUCTOR,
}

/// System actor state.
#[derive(Default, Deserialize_tuple, Serialize_tuple, Debug, Clone)]
pub struct State {
    // builtin actor registry: Vec<(String, Cid)>
    pub builtin_actors: Cid,
}

impl State {
    pub fn new<BS: Blockstore>(store: &BS) -> Result<Self, ActorError> {
        let c = store
            .put_cbor(&Vec::<(String, Cid)>::new(), multihash::Code::Blake2b256)
            .context_code(ExitCode::USR_ILLEGAL_STATE, "failed to store system state")?;
        Ok(Self { builtin_actors: c })
    }

    pub fn get_builtin_actors<B: Blockstore>(
        &self,
        store: &B,
    ) -> Result<Vec<(String, Cid)>, String> {
        match store.get_cbor(&self.builtin_actors) {
            Ok(Some(obj)) => Ok(obj),
            Ok(None) => Err("failed to load builtin actor registry; not found".to_string()),
            Err(e) => Err(e.to_string()),
        }
    }
}

/// System actor.
pub struct Actor;

impl Actor {
    /// System actor constructor.
    pub fn constructor(rt: &impl Runtime) -> Result<(), ActorError> {
        rt.validate_immediate_caller_is(std::iter::once(&SYSTEM_ACTOR_ADDR))?;

        let state = State::new(rt.store()).context("failed to construct state")?;
        rt.create(&state)?;
        Ok(())
    }
}

impl ActorCode for Actor {
    type Methods = Method;

    fn name() -> &'static str {
        "System"
    }

    actor_dispatch! {
        Constructor => constructor,
    }
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;

    use fvm_shared::MethodNum;

    use fil_actors_runtime::test_utils::{MockRuntime, SYSTEM_ACTOR_CODE_ID};
    use fil_actors_runtime::SYSTEM_ACTOR_ADDR;

    use crate::{Actor, Method, State};

    pub fn new_runtime() -> MockRuntime {
        MockRuntime {
            receiver: SYSTEM_ACTOR_ADDR,
            caller: RefCell::new(SYSTEM_ACTOR_ADDR),
            caller_type: RefCell::new(*SYSTEM_ACTOR_CODE_ID),
            ..Default::default()
        }
    }

    #[test]
    fn construct_with_root_id() {
        let rt = new_runtime();
        rt.expect_validate_caller_addr(vec![SYSTEM_ACTOR_ADDR]);
        rt.set_caller(*SYSTEM_ACTOR_CODE_ID, SYSTEM_ACTOR_ADDR);
        rt.call::<Actor>(Method::Constructor as MethodNum, None).unwrap();

        let state: State = rt.get_state();
        let builtin_actors = state.get_builtin_actors(&rt.store).unwrap();
        assert!(builtin_actors.is_empty());
    }
}
