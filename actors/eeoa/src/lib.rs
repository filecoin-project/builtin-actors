use fvm_ipld_encoding::{Cbor, RawBytes};
use fvm_shared::address::Payload;
use fvm_shared::error::ExitCode;
use fvm_shared::{MethodNum, METHOD_CONSTRUCTOR};
use num_derive::FromPrimitive;
use num_traits::FromPrimitive;
use serde::Deserialize;
use serde::Serialize;

use fil_actors_runtime::runtime::{ActorCode, Runtime};
use fil_actors_runtime::{
    actor_error, restrict_internal_api, ActorError, AsActorError, EAM_ACTOR_ID, SYSTEM_ACTOR_ADDR,
};

#[cfg(feature = "fil-actor")]
fil_actors_runtime::wasm_trampoline!(EeoaActor);

/// Ethereum Externally Owned Address actor methods.
#[derive(FromPrimitive)]
#[repr(u64)]
pub enum Method {
    Constructor = METHOD_CONSTRUCTOR,
}

/// Ethereum Externally Owned Address actor state.
#[derive(Default, Deserialize, Serialize)]
#[serde(transparent)]
pub struct State([(); 0]);

impl Cbor for State {}

/// Ethereum Externally Owned Address actor.
pub struct EeoaActor;

impl EeoaActor {
    /// Ethereum Externally Owned Address actor constructor.
    pub fn constructor(rt: &mut impl Runtime) -> Result<(), ActorError> {
        rt.validate_immediate_caller_is(std::iter::once(&SYSTEM_ACTOR_ADDR))?;

        let valid = rt
            .lookup_address(rt.message().receiver().id().unwrap())
            .map(
                |a| matches!(a.payload(), Payload::Delegated(da) if da.namespace() == EAM_ACTOR_ID),
            )
            .with_context_code(ExitCode::USR_ILLEGAL_ARGUMENT, || {
                "receiver must have predictable address".to_string()
            })?;
        if !valid {
            return Err(ActorError::illegal_argument(
                "invalid target for EEOA creation".to_string(),
            ));
        }

        rt.create(&State::default())?;
        Ok(())
    }
}

impl ActorCode for EeoaActor {
    fn invoke_method<RT>(
        rt: &mut RT,
        method: MethodNum,
        _params: &RawBytes,
    ) -> Result<RawBytes, ActorError>
    where
        RT: Runtime,
    {
        restrict_internal_api(rt, method)?;
        match FromPrimitive::from_u64(method) {
            Some(Method::Constructor) => {
                Self::constructor(rt)?;
                Ok(RawBytes::default())
            }
            None => Err(actor_error!(unhandled_message; "Invalid method")),
        }
    }
}

#[cfg(test)]
mod tests {
    use fil_actors_runtime::EAM_ACTOR_ID;
    use fvm_ipld_encoding::RawBytes;
    use fvm_shared::address::Address;
    use fvm_shared::error::ExitCode;
    use fvm_shared::MethodNum;

    use fil_actors_runtime::test_utils::{
        expect_abort_contains_message, MockRuntime, SYSTEM_ACTOR_CODE_ID,
    };
    use fil_actors_runtime::SYSTEM_ACTOR_ADDR;

    use crate::{EeoaActor, Method, State};

    const EOA: Address = Address::new_id(1000);

    pub fn new_runtime() -> MockRuntime {
        MockRuntime {
            receiver: EOA,
            caller: SYSTEM_ACTOR_ADDR,
            caller_type: *SYSTEM_ACTOR_CODE_ID,
            ..Default::default()
        }
    }

    #[test]
    fn construct_from_system() {
        let mut rt = new_runtime();
        rt.expect_validate_caller_addr(vec![SYSTEM_ACTOR_ADDR]);
        rt.set_caller(*SYSTEM_ACTOR_CODE_ID, SYSTEM_ACTOR_ADDR);
        rt.add_delegated_address(
            EOA,
            Address::new_delegated(
                EAM_ACTOR_ID,
                &hex_literal::hex!("FEEDFACECAFEBEEF000000000000000000000000"),
            )
            .unwrap(),
        );
        rt.call::<EeoaActor>(Method::Constructor as MethodNum, &RawBytes::default()).unwrap();
        rt.verify();
        let state: State = rt.get_state();
        assert_eq!([(); 0], state.0);
    }

    #[test]
    fn no_delegated_cant_deploy() {
        let mut rt = new_runtime();
        rt.expect_validate_caller_addr(vec![SYSTEM_ACTOR_ADDR]);
        rt.set_caller(*SYSTEM_ACTOR_CODE_ID, SYSTEM_ACTOR_ADDR);
        expect_abort_contains_message(
            ExitCode::USR_ILLEGAL_ARGUMENT,
            "receiver must have predictable address",
            rt.call::<EeoaActor>(Method::Constructor as MethodNum, &RawBytes::default()),
        );
        rt.verify();
    }
}
