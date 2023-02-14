pub mod types;

use fvm_ipld_encoding::ipld_block::IpldBlock;
use fvm_shared::address::{Payload, Protocol};
use fvm_shared::crypto::hash::SupportedHashes::Keccak256;
use fvm_shared::error::ExitCode;
use fvm_shared::{MethodNum, METHOD_CONSTRUCTOR};
use num_derive::FromPrimitive;

use crate::types::AuthenticateMessageParams;
use fil_actors_runtime::runtime::{ActorCode, Runtime};
use fil_actors_runtime::{
    actor_dispatch, actor_error, ActorError, AsActorError, EAM_ACTOR_ID,
    FIRST_EXPORTED_METHOD_NUMBER, SYSTEM_ACTOR_ADDR,
};

#[cfg(feature = "fil-actor")]
fil_actors_runtime::wasm_trampoline!(EthAccountActor);

/// Ethereum Account actor methods.
#[derive(FromPrimitive)]
#[repr(u64)]
pub enum Method {
    Constructor = METHOD_CONSTRUCTOR,
    AuthenticateMessageExported = frc42_dispatch::method_hash!("AuthenticateMessage"),
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

    /// Authenticates whether the provided signature is valid for the provided message.
    /// Should be called with the raw bytes of a signature, NOT a serialized Signature object that includes a SignatureType.
    /// Errors with USR_ILLEGAL_ARGUMENT if the authentication is invalid.
    pub fn authenticate_message(
        rt: &mut impl Runtime,
        params: AuthenticateMessageParams,
    ) -> Result<(), ActorError> {
        rt.validate_immediate_caller_accept_any()?;
        let msg_hash = rt.hash_blake2b(&params.message);

        let signer_pk = rt
            .recover_secp_public_key(
                &msg_hash,
                params
                    .signature
                    .as_slice()
                    .try_into()
                    .map_err(|_| actor_error!(illegal_argument; "invalid signature length"))?,
            )
            .with_context_code(ExitCode::USR_ILLEGAL_ARGUMENT, || {
                "failed to recover signer public key"
            })?;

        // 0x04 to indicate uncompressed point
        if signer_pk[0] != 0x04 {
            return Err(actor_error!(assertion_failed; "pubkey should start with 0x04, not {}",
                signer_pk[0]));
        }

        // The subaddress is the last 20 bytes of the keccak hash of the public key
        let signer_pk_hash = rt.hash(Keccak256, &signer_pk[1..]);
        if signer_pk_hash.len() < 20 {
            return Err(
                actor_error!(assertion_failed; "invalid keccak hash length {}", signer_pk_hash.len()),
            );
        }

        let signer_subaddress_bytes = &signer_pk_hash[signer_pk_hash.len() - 20..];

        let self_address = rt
            .lookup_delegated_address(
                rt.message().receiver().id().expect("receiver must be ID address"),
            )
            .context_code(
                ExitCode::USR_ILLEGAL_STATE,
                "ethaccount should always have delegated address",
            )?;

        let self_address_bytes = self_address.to_bytes();
        if self_address_bytes[0] != Protocol::Delegated as u8
            || self_address_bytes[1] != EAM_ACTOR_ID as u8
        {
            return Err(actor_error!(illegal_state;
                    "first 2 bytes of f4 address payload weren't Delegated protocol {} and EAM address {}",
                    self_address_bytes[0],
                    self_address_bytes[1]));
        }

        // drop the first 2 bytes (protocol and EAM namespace)
        let self_subaddress_bytes = &self_address_bytes[2..];

        if self_subaddress_bytes != signer_subaddress_bytes {
            return Err(actor_error!(illegal_argument; "invalid signature for {}", self_address));
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
        AuthenticateMessageExported => authenticate_message,
        _ => fallback [raw],
    }
}
