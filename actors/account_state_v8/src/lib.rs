// Copyright 2019-2022 ChainSafe Systems
// SPDX-License-Identifier: Apache-2.0, MIT

use fvm_ipld_encoding::RawBytes;
use fvm_shared::address::{Address, Protocol};
use fvm_shared::{MethodNum, METHOD_CONSTRUCTOR};
use num_derive::FromPrimitive;
use num_traits::FromPrimitive;

use fil_actors_runtime_common::builtin::singletons::SYSTEM_ACTOR_ADDR;
use fil_actors_runtime_common::cbor;
use fil_actors_runtime_common::runtime::{ActorCode, Runtime};
use fil_actors_runtime_common::{actor_error, ActorError};

pub use self::state::State;

mod state;
pub mod testing;

#[cfg(feature = "fil-actor")]
fil_actors_runtime_common::wasm_trampoline!(Actor);

// * Updated to specs-actors commit: 845089a6d2580e46055c24415a6c32ee688e5186 (v3.0.0)

/// Account actor methods available
#[derive(FromPrimitive)]
#[repr(u64)]
pub enum Method {
    Constructor = METHOD_CONSTRUCTOR,
    PubkeyAddress = 2,
}

/// Account Actor
pub struct Actor;
impl Actor {
    /// Constructor for Account actor
    pub fn constructor<RT>(rt: &mut RT, address: Address) -> Result<(), ActorError>
    where
        RT: Runtime,
    {
        rt.validate_immediate_caller_is(std::iter::once(&SYSTEM_ACTOR_ADDR))?;
        match address.protocol() {
            Protocol::Secp256k1 | Protocol::BLS => {}
            protocol => {
                return Err(actor_error!(illegal_argument;
                    "address must use BLS or SECP protocol, got {}", protocol));
            }
        }
        rt.create(&State { address })?;
        Ok(())
    }

    // Fetches the pubkey-type address from this actor.
    pub fn pubkey_address<RT>(rt: &mut RT) -> Result<Address, ActorError>
    where
        RT: Runtime,
    {
        rt.validate_immediate_caller_accept_any()?;
        let st: State = rt.state()?;
        Ok(st.address)
    }
}

impl ActorCode for Actor {
    fn invoke_method<RT>(
        rt: &mut RT,
        method: MethodNum,
        params: &RawBytes,
    ) -> Result<RawBytes, ActorError>
    where
        RT: Runtime,
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
            None => Err(actor_error!(unhandled_message; "Invalid method")),
        }
    }
}
