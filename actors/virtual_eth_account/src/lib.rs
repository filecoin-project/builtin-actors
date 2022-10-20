use fil_actors_runtime::runtime::builtins::Type;
use fvm_ipld_blockstore::Blockstore;
use fvm_ipld_encoding::RawBytes;
use fvm_shared::address::{Address, Payload};
use fvm_shared::{MethodNum, METHOD_CONSTRUCTOR};
use num_derive::FromPrimitive;
use num_traits::FromPrimitive;

use fil_actors_runtime::runtime::{ActorCode, Runtime};
use fil_actors_runtime::{actor_error, ActorError};
use fil_actors_runtime::cbor;
use types::{ForwardParams};

pub use self::state::State;

mod state;
// pub mod testing;
pub mod types;

#[cfg(feature = "fil-actor")]
fil_actors_runtime::wasm_trampoline!(Actor);

/// Account actor methods available
#[derive(FromPrimitive)]
#[repr(u64)]
pub enum Method {
    Constructor = METHOD_CONSTRUCTOR,
    PubkeyAddress = 2,
    ForwardMessage = 3,
}

/// virtual ETH account for a native f1 account 
pub struct Actor;

impl Actor {
    // TODO FIP 0044
    // https://github.com/filecoin-project/ref-fvm/issues/994

    /// Constructor for Account actor
    pub fn constructor<BS, RT>(rt: &mut RT, address: Address) -> Result<(), ActorError>
    where
        BS: Blockstore,
        RT: Runtime<BS>,
    {
        rt.resolve_address(&address)
            .ok_or(actor_error!(illegal_argument; "actor must already exist"))?;

        match address.payload() {
            Payload::Secp256k1(_) => (),
            _ => {
                return Err(ActorError::illegal_argument(
                    "Constructing actor must be an f1 account (secp256k1)".into(),
                ))
            }
        }

        // TODO this is always the Init actor ATM, due to how exec4 works but really its the EAM actor
        rt.validate_immediate_caller_type(std::iter::once(&Type::Init))?;
        rt.create(&State { manager: address })?;
        Ok(())
    }

    /// Fetches the pubkey-type address from this actor. Always f1
    pub fn pubkey_address<BS, RT>(rt: &mut RT) -> Result<Address, ActorError>
    where
        BS: Blockstore,
        RT: Runtime<BS>,
    {
        rt.validate_immediate_caller_accept_any()?;
        let st: State = rt.state()?;
        Ok(st.manager)
    }

    fn forward_msg<BS, RT>(rt: &mut RT, fwd: ForwardParams) -> Result<RawBytes, ActorError>
    where
        BS: Blockstore,
        RT: Runtime<BS>,
    {
        let st: State = rt.state()?;
        rt.validate_immediate_caller_is(std::iter::once(&st.manager))?;

        let ForwardParams { to, method, params, value } = fwd;
        rt.send(&to, method, RawBytes::new(params), value)
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
            Some(Method::PubkeyAddress) => {
                let addr = Self::pubkey_address(rt)?;
                Ok(RawBytes::serialize(addr)?)
            }
            Some(Method::ForwardMessage) => {
                let ret = Self::forward_msg(rt, cbor::deserialize_params(params)?)?;
                Ok(RawBytes::serialize(ret)?)
            }
            None => Err(actor_error!(unhandled_message; "Invalid method")),
        }
    }
}
