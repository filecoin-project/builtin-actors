pub mod types;

use fvm_ipld_encoding::ipld_block::IpldBlock;
use fvm_shared::address::Payload;
use fvm_shared::{MethodNum, METHOD_CONSTRUCTOR};
use num_derive::FromPrimitive;

use fil_actors_runtime::runtime::{ActorCode, Runtime};
use fil_actors_runtime::{
    actor_dispatch, actor_error, ActorError, EAM_ACTOR_ID, FIRST_EXPORTED_METHOD_NUMBER,
    SYSTEM_ACTOR_ADDR,
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

    // Always succeeds, accepting any transfers.
    pub fn fallback(
        rt: &mut impl Runtime,
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

impl ActorCode for EthAccountActor {
    type Methods = Method;
    actor_dispatch! {
        Constructor => constructor,
        _ => fallback [raw],
    }
}
