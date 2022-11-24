// Copyright 2019-2022 ChainSafe Systems
// SPDX-License-Identifier: Apache-2.0, MIT

use fvm_ipld_encoding::RawBytes;
use fvm_shared::address::{Address, Protocol};
use fvm_shared::crypto::signature::SignatureType::{Secp256k1, BLS};
use fvm_shared::crypto::signature::{Signature, SignatureType};
use fvm_shared::error::ExitCode;
use fvm_shared::{MethodNum, METHOD_CONSTRUCTOR};
use num_derive::FromPrimitive;
use num_traits::FromPrimitive;

use fil_actors_runtime::builtin::singletons::SYSTEM_ACTOR_ADDR;
use fil_actors_runtime::runtime::{ActorCode, Runtime};
use fil_actors_runtime::ActorDowncast;
use fil_actors_runtime::{actor_error, decode_params, ActorError, AsActorError};
use fvm_shared::ipld_block::IpldBlock;

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
    AuthenticateMessage = 3,
    UniversalReceiverHook = frc42_dispatch::method_hash!("Receive"),
}

/// Account Actor
pub struct Actor;

impl Actor {
    /// Constructor for Account actor
    pub fn constructor(rt: &mut impl Runtime, args: Option<IpldBlock>) -> Result<(), ActorError> {
        let params: Address = decode_params!(args);

        rt.validate_immediate_caller_is(std::iter::once(&SYSTEM_ACTOR_ADDR))?;
        match params.protocol() {
            Protocol::Secp256k1 | Protocol::BLS => {}
            protocol => {
                return Err(actor_error!(illegal_argument;
                    "address must use BLS or SECP protocol, got {}", protocol));
            }
        }
        rt.create(&State { address: params })?;
        Ok(())
    }

    /// Fetches the pubkey-type address from this actor.
    pub fn pubkey_address(
        rt: &mut impl Runtime,
        _args: Option<IpldBlock>,
    ) -> Result<Address, ActorError> {
        // TODO: NO_PARAMS
        rt.validate_immediate_caller_accept_any()?;
        let st: State = rt.state()?;
        Ok(st.address)
    }

    /// Authenticates whether the provided signature is valid for the provided message.
    /// Should be called with the raw bytes of a signature, NOT a serialized Signature object that includes a SignatureType.
    /// Errors with USR_ILLEGAL_ARGUMENT if the authentication is invalid.
    pub fn authenticate_message(
        rt: &mut impl Runtime,
        args: Option<IpldBlock>,
    ) -> Result<(), ActorError> {
        let params: AuthenticateMessageParams = decode_params!(args);

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

        Ok(())
    }

    // Always succeeds, accepting any transfers.
    pub fn universal_receiver_hook(
        rt: &mut impl Runtime,
        _args: Option<IpldBlock>,
    ) -> Result<(), ActorError> {
        // TODO: NO_PARAMS
        rt.validate_immediate_caller_accept_any()?;
        Ok(())
    }
}

impl ActorCode for Actor {
    fn invoke_method<RT>(
        rt: &mut RT,
        method: MethodNum,
        args: Option<IpldBlock>,
    ) -> Result<RawBytes, ActorError>
    where
        RT: Runtime,
    {
        match FromPrimitive::from_u64(method) {
            Some(Method::Constructor) => {
                Self::constructor(rt, args)?;
                Ok(RawBytes::default())
            }
            Some(Method::PubkeyAddress) => {
                let addr = Self::pubkey_address(rt, args)?;
                Ok(RawBytes::serialize(addr)?)
            }
            Some(Method::AuthenticateMessage) => {
                Self::authenticate_message(rt, args)?;
                Ok(RawBytes::default())
            }
            Some(Method::UniversalReceiverHook) => {
                Self::universal_receiver_hook(rt, args)?;
                Ok(RawBytes::default())
            }
            None => Err(actor_error!(unhandled_message; "Invalid method")),
        }
    }
}
