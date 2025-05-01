// Copyright 2025 Curio Storage Inc.
// SPDX-License-Identifier: Apache-2.0, MIT

use fvm_ipld_encoding::ipld_block::IpldBlock;
use fvm_shared::{METHOD_CONSTRUCTOR, MethodNum};

use num_derive::FromPrimitive;

use fil_actors_runtime::builtin::singletons::INIT_ACTOR_ADDR;
use fil_actors_runtime::runtime::{ActorCode, Runtime};
use fil_actors_runtime::{FIRST_EXPORTED_METHOD_NUMBER, actor_dispatch};
use fil_actors_runtime::{ActorError, ActorDowncast, actor_error};
use fvm_ipld_bitfield::{BitField, Validate};
use fvm_ipld_encoding::{CborStore};
use multihash_codetable::Code;
use fvm_shared::error::ExitCode;
use fil_actors_runtime::runtime::builtins::Type;
use fvm_shared::sector::SectorNumber;
use fil_actors_runtime::runtime::policy_constants::MAX_SECTOR_NUMBER;
use fvm_shared::econ::TokenAmount;
use num_traits::Zero;
use fvm_shared::sys::SendFlags;

use crate::types::{ConstructorParams, ActivateSectorParams, CompactSectorNumbersParams, ActivateSectorReturn};
use crate::ext::account;
pub use self::state::{State, CollisionPolicy};

pub mod ext;
mod state;
pub mod types;
pub mod testing;

#[cfg(feature = "fil-actor")]
fil_actors_runtime::wasm_trampoline!(Actor);

/// Sealer actor methods available
#[derive(FromPrimitive)]
#[repr(u64)]
pub enum Method {
    Constructor = METHOD_CONSTRUCTOR,
    
    ActivateSectors = frc42_dispatch::method_hash!("ActivateSectors"),
    CompactSectorNumbers = frc42_dispatch::method_hash!("CompactSectorNumbers"),
}

/// Sealer Actor
pub struct Actor;

impl Actor {
    /// Constructor for Sealer actor
    pub fn constructor(rt: &impl Runtime, _params: ConstructorParams) -> Result<(), ActorError> {
        rt.validate_immediate_caller_is(std::iter::once(&INIT_ACTOR_ADDR))?;

        let empty_bitfield = rt.store().put_cbor(&BitField::new(), Code::Blake2b256).map_err(|e| {
            e.downcast_default(ExitCode::USR_ILLEGAL_STATE, "failed to construct empty bitfield")
        })?;

        let validator = _params.validator;
        let state = State {
            validator,
            allocated_sectors: empty_bitfield,
        };
        rt.create(&state)?;
        Ok(())
    }

    pub fn activate_sectors(rt: &impl Runtime, params: ActivateSectorParams) -> Result<ActivateSectorReturn, ActorError> {
        rt.validate_immediate_caller_type(std::iter::once(&Type::Miner))?;

        rt.transaction(|state: &mut State, rt| {

            // Call the validator with the sector numbers
            let payload = types::VerifierSignaturePayload::new(
                params.sector_numbers.clone(),
                rt.message().receiver(),
                rt.message().caller(),
            );
            
            let serialized_payload = payload.serialize()
                .map_err(|e| actor_error!(illegal_state, "failed to serialize payload: {}", e))?;
            
            // We're not actually signing anything here, just passing the payload to the validator
            // The validator will verify the sector numbers are valid
            let auth_params = account::AuthenticateMessageParams {
                signature: params.verifier_signature,
                message: serialized_payload,
            };
            
            // Call the validator actor to authenticate the sector numbers
            let send_flags = SendFlags::default();

            rt.send(
                &state.validator,
                account::AUTHENTICATE_MESSAGE_METHOD,
                IpldBlock::serialize_cbor(&auth_params)?,
                TokenAmount::zero(),
                None,
                send_flags,
            )?;
            
            // Allocate the sector numbers after validation
            state.allocate_sector_numbers(
                rt.store(),
                &params.sector_numbers,
                CollisionPolicy::DenyCollisions,
            )
        })?;

        Ok(ActivateSectorReturn {
            sector_numbers: params.sector_numbers,
        })
    }
    
    pub fn compact_sector_numbers(rt: &impl Runtime, params: CompactSectorNumbersParams) -> Result<(), ActorError> {
        let mask_sector_numbers = params
            .mask_sector_numbers
            .validate()
            .map_err(|e| actor_error!(illegal_argument, "invalid mask bitfield: {}", e))?;

        let last_sector_number = mask_sector_numbers
            .last()
            .ok_or_else(|| actor_error!(illegal_argument, "invalid mask bitfield"))?
            as SectorNumber;

        if last_sector_number > MAX_SECTOR_NUMBER {
            return Err(actor_error!(
                illegal_argument,
                "masked sector number {} exceeded max sector number",
                last_sector_number
            ));
        }

        rt.transaction(|state: &mut State, rt| {
            rt.validate_immediate_caller_is([state.validator].iter())?;

            state.allocate_sector_numbers(
                rt.store(),
                mask_sector_numbers,
                CollisionPolicy::AllowCollisions,
            )
        })?;
        
        Ok(())
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
        "Sealer"
    }

    actor_dispatch! {
        Constructor => constructor,
        ActivateSectors => activate_sectors,
        CompactSectorNumbers => compact_sector_numbers,
        _ => fallback,
    }
}
