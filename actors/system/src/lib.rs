// Copyright 2019-2022 ChainSafe Systems
// SPDX-License-Identifier: Apache-2.0, MIT
use cid::Cid;
use fvm_shared::blockstore::Blockstore;
use fvm_shared::encoding::{Cbor, RawBytes};
use fvm_shared::{MethodNum, METHOD_CONSTRUCTOR};
use num_derive::FromPrimitive;
use num_traits::FromPrimitive;
use serde::{Deserialize, Serialize};

use fil_actors_runtime::runtime::{ActorCode, Runtime};
use fil_actors_runtime::{actor_error, wasm_trampoline, ActorError, SYSTEM_ACTOR_ADDR};

wasm_trampoline!(Actor);

// * Updated to specs-actors commit: 845089a6d2580e46055c24415a6c32ee688e5186 (v3.0.0)

/// System actor methods.
#[derive(FromPrimitive)]
#[repr(u64)]
pub enum Method {
    Constructor = METHOD_CONSTRUCTOR,
}

/// System actor state.
#[derive(Default, Deserialize, Serialize)]
#[serde(transparent)]
pub struct State {
    builtin_actors: Cid
}
impl Cbor for State {}

/// System actor.
pub struct Actor;
impl Actor {
    /// System actor constructor.
    pub fn constructor<BS, RT>(rt: &mut RT) -> Result<(), ActorError>
    where
        BS: Blockstore,
        RT: Runtime<BS>,
    {
        rt.validate_immediate_caller_is(std::iter::once(&*SYSTEM_ACTOR_ADDR))?;

        rt.create(&State::default())?;
        Ok(())
    }
}

impl ActorCode for Actor {
    fn invoke_method<BS, RT>(
        rt: &mut RT,
        method: MethodNum,
        _params: &RawBytes,
    ) -> Result<RawBytes, ActorError>
    where
        BS: Blockstore,
        RT: Runtime<BS>,
    {
        match FromPrimitive::from_u64(method) {
            Some(Method::Constructor) => {
                Self::constructor(rt)?;
                Ok(RawBytes::default())
            }
            None => Err(actor_error!(SysErrInvalidMethod; "Invalid method")),
        }
    }
}

#[cfg(test)]
mod tests {
    use fvm_shared::encoding::RawBytes;
    use fvm_shared::MethodNum;

    use fil_actors_runtime::test_utils::{MockRuntime, SYSTEM_ACTOR_CODE_ID};
    use fil_actors_runtime::SYSTEM_ACTOR_ADDR;

    use crate::{Actor, Method, State, Cid};

    pub fn new_runtime() -> MockRuntime {
        MockRuntime {
            receiver: *SYSTEM_ACTOR_ADDR,
            caller: *SYSTEM_ACTOR_ADDR,
            caller_type: *SYSTEM_ACTOR_CODE_ID,
            ..Default::default()
        }
    }

    #[test]
    fn construct_with_root_id() {
        let mut rt = new_runtime();
        rt.expect_validate_caller_addr(vec![*SYSTEM_ACTOR_ADDR]);
        rt.set_caller(*SYSTEM_ACTOR_CODE_ID, *SYSTEM_ACTOR_ADDR);
        rt.call::<Actor>(Method::Constructor as MethodNum, &RawBytes::default()).unwrap();

        let state: State = rt.get_state().unwrap();
        assert_eq!(state.builtin_actors, Cid::default());
    }
}
