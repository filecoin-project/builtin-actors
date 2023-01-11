use fvm_shared::address::Payload;
use fvm_shared::{MethodNum, METHOD_CONSTRUCTOR};
use num_derive::FromPrimitive;
use num_traits::FromPrimitive;

use fil_actors_runtime::runtime::{ActorCode, Runtime};
use fil_actors_runtime::{
    actor_dispatch, actor_error, restrict_internal_api, ActorError, EAM_ACTOR_ID, SYSTEM_ACTOR_ADDR,
};

#[cfg(feature = "fil-actor")]
fil_actors_runtime::wasm_trampoline!(EthAccountActor);

/// Ethereum Account actor methods.
#[derive(FromPrimitive)]
#[repr(u64)]
pub enum Method {
    Constructor = METHOD_CONSTRUCTOR,
}

/// Ethereum Account actor.
pub struct EthAccountActor;

impl EthAccountActor {
    /// Ethereum Account actor constructor.
    /// NOTE: This method is NOT currently called from anywhere, instead the FVM just deploys EthAccounts.
    pub fn constructor(rt: &mut impl Runtime) -> Result<(), ActorError> {
        rt.validate_immediate_caller_is(std::iter::once(&SYSTEM_ACTOR_ADDR))?;

        match rt
            .lookup_delegated_address(rt.message().receiver().id().unwrap())
            .map(|a| *a.payload())
        {
            Some(Payload::Delegated(da)) if da.namespace() == EAM_ACTOR_ID => {}
            Some(_) => {
                return Err(ActorError::illegal_argument(
                    "invalid target for EthAccount creation".to_string(),
                ));
            }
            None => {
                return Err(ActorError::illegal_argument(
                    "receiver must have a predictable address".to_string(),
                ));
            }
        }

        Ok(())
    }
}

impl ActorCode for EthAccountActor {
    type Methods = Method;
    actor_dispatch! {
        Constructor => constructor,
    }
}

#[cfg(test)]
mod tests {
    use fil_actors_runtime::EAM_ACTOR_ID;
    use fvm_shared::address::Address;
    use fvm_shared::error::ExitCode;
    use fvm_shared::MethodNum;

    use fil_actors_runtime::test_utils::{
        expect_abort_contains_message, MockRuntime, SYSTEM_ACTOR_CODE_ID,
    };
    use fil_actors_runtime::SYSTEM_ACTOR_ADDR;

    use crate::{EthAccountActor, Method};

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
        rt.call::<EthAccountActor>(Method::Constructor as MethodNum, None).unwrap();
        rt.verify();
    }

    #[test]
    fn no_delegated_cant_deploy() {
        let mut rt = new_runtime();
        rt.expect_validate_caller_addr(vec![SYSTEM_ACTOR_ADDR]);
        rt.set_caller(*SYSTEM_ACTOR_CODE_ID, SYSTEM_ACTOR_ADDR);
        expect_abort_contains_message(
            ExitCode::USR_ILLEGAL_ARGUMENT,
            "receiver must have a predictable address",
            rt.call::<EthAccountActor>(Method::Constructor as MethodNum, None),
        );
        rt.verify();
    }
}
