// Copyright 2019-2022 ChainSafe Systems
// SPDX-License-Identifier: Apache-2.0, MIT

use fvm_ipld_encoding::ipld_block::IpldBlock;
use fvm_shared::address::Protocol;
use fvm_shared::crypto::signature::SignatureType::{Secp256k1, BLS};
use fvm_shared::crypto::signature::{Signature, SignatureType};
use fvm_shared::error::ExitCode;
use fvm_shared::{MethodNum, METHOD_CONSTRUCTOR};
use num_derive::FromPrimitive;

use fil_actors_runtime::builtin::singletons::SYSTEM_ACTOR_ADDR;
use fil_actors_runtime::runtime::{ActorCode, Runtime};
use fil_actors_runtime::{actor_dispatch, ActorDowncast, FIRST_EXPORTED_METHOD_NUMBER};
use fil_actors_runtime::{actor_error, ActorError};
use types::{AuthenticateMessageReturn, ConstructorParams, PubkeyAddressReturn};

use crate::types::AuthenticateMessageParams;

pub use self::state::State;

mod state;
pub mod testing;
pub mod types;

#[cfg(feature = "fil-actor")]
fil_actors_runtime::wasm_trampoline!(Actor);

/// Account actor methods available
#[derive(FromPrimitive)]
#[repr(u64)]
pub enum Method {
    Constructor = METHOD_CONSTRUCTOR,
    PubkeyAddress = 2,
    // Deprecated in v10
    // AuthenticateMessage = 3,
    AuthenticateMessageExported = frc42_dispatch::method_hash!("AuthenticateMessage"),
}

/// Account Actor
pub struct Actor;

impl Actor {
    /// Constructor for Account actor
    pub fn constructor(rt: &impl Runtime, params: ConstructorParams) -> Result<(), ActorError> {
        let address = params.address;
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

    /// Fetches the pubkey-type address from this actor.
    pub fn pubkey_address(rt: &impl Runtime) -> Result<PubkeyAddressReturn, ActorError> {
        rt.validate_immediate_caller_accept_any()?;
        let st: State = rt.state()?;
        Ok(PubkeyAddressReturn { address: st.address })
    }

    /// Authenticates whether the provided signature is valid for the provided message.
    /// Should be called with the raw bytes of a signature, NOT a serialized Signature object that includes a SignatureType.
    /// Errors with USR_ILLEGAL_ARGUMENT if the authentication is invalid.
    pub fn authenticate_message(
        rt: &impl Runtime,
        params: AuthenticateMessageParams,
    ) -> Result<AuthenticateMessageReturn, ActorError> {
        rt.validate_immediate_caller_accept_any()?;
        let st: State = rt.state()?;
        let address = st.address;
        let sig_type: SignatureType = match address.protocol() {
            Protocol::Secp256k1 => Secp256k1,
            Protocol::BLS => BLS,
            protocol => {
                return Err(actor_error!(illegal_state;
                    "account address must use BLS or SECP protocol, got {}", protocol));
            }
        };
        let sig = Signature { sig_type, bytes: params.signature };
        rt.verify_signature(&sig, &address, &params.message).map_err(|e| {
            e.downcast_default(
                ExitCode::USR_ILLEGAL_ARGUMENT,
                "failed to authenticate message, signature invalid",
            )
        })?;

        Ok(AuthenticateMessageReturn { authenticated: true })
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
        "Account"
    }

    actor_dispatch! {
        Constructor => constructor,
        PubkeyAddress => pubkey_address,
        AuthenticateMessageExported => authenticate_message,
        _ => fallback [raw],
    }
}
