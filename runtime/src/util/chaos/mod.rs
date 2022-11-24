// Copyright 2019-2022 ChainSafe Systems
// SPDX-License-Identifier: Apache-2.0, MIT

use cid::Cid;
use fvm_ipld_encoding::RawBytes;
use fvm_shared::address::Address;
use fvm_shared::error::ExitCode;
use fvm_shared::ipld_block::IpldBlock;
use fvm_shared::{MethodNum, METHOD_CONSTRUCTOR};
use num_derive::FromPrimitive;
use num_traits::FromPrimitive;
pub use state::*;
pub use types::*;

use crate::runtime::builtins::Type;
use crate::runtime::{ActorCode, Runtime};
use crate::{actor_error, decode_params, ActorError, AsActorError};

mod state;
mod types;
mod unmarshallable;

// * Updated to test-vectors commit: 907892394dd83fe1f4bf1a82146bbbcc58963148

// Caller Validation methods
const CALLER_VALIDATION_BRANCH_NONE: i64 = 0;
const CALLER_VALIDATION_BRANCH_TWICE: i64 = 1;
const CALLER_VALIDATION_BRANCH_IS_ADDRESS: i64 = 2;
const CALLER_VALIDATION_BRANCH_IS_TYPE: i64 = 3;

// Mutate State Branch Methods
const MUTATE_IN_TRANSACTION: i64 = 0;

/// Chaos actor methods available
#[derive(FromPrimitive)]
#[repr(u64)]
pub enum Method {
    Constructor = METHOD_CONSTRUCTOR,
    CallerValidation = 2,
    CreateActor = 3,
    ResolveAddress = 4,
    DeleteActor = 5,
    Send = 6,
    MutateState = 7,
    AbortWith = 8,
    InspectRuntime = 9,
}

/// Chaos Actor
pub struct Actor;

impl Actor {
    pub fn send(rt: &mut impl Runtime, args: Option<IpldBlock>) -> Result<SendReturn, ActorError> {
        let params: SendArgs = decode_params!(args);
        rt.validate_immediate_caller_accept_any()?;

        let result = rt.send(&params.to, params.method, params.params, params.value);
        if let Err(e) = result {
            Ok(SendReturn { return_value: RawBytes::default(), code: e.exit_code() })
        } else {
            Ok(SendReturn { return_value: result.unwrap(), code: ExitCode::OK })
        }
    }

    /// Constructor for Chaos actor
    pub fn constructor(_rt: &mut impl Runtime, _params: Option<IpldBlock>) {
        panic!("Constructor should not be called");
    }

    /// CallerValidation violates VM call validation constraints.
    ///
    ///  CALLER_VALIDATION_BRANCH_NONE performs no validation.
    ///  CALLER_VALIDATION_BRANCH_TWICE validates twice.
    ///  CALLER_VALIDATION_BRANCH_IS_ADDRESS validates against an empty caller
    ///  address set.
    ///  CALLER_VALIDATION_BRANCH_IS_TYPE validates against an empty caller type set.
    pub fn caller_validation(
        rt: &mut impl Runtime,
        args: Option<IpldBlock>,
    ) -> Result<(), ActorError> {
        let params: CallerValidationArgs = decode_params!(args);

        match params.branch {
            x if x == CALLER_VALIDATION_BRANCH_NONE => {}
            x if x == CALLER_VALIDATION_BRANCH_TWICE => {
                rt.validate_immediate_caller_accept_any()?;
                rt.validate_immediate_caller_accept_any()?;
            }
            x if x == CALLER_VALIDATION_BRANCH_IS_ADDRESS => {
                rt.validate_immediate_caller_is(&params.addrs)?;
            }
            x if x == CALLER_VALIDATION_BRANCH_IS_TYPE => {
                let types: Vec<Type> = params
                    .types
                    .iter()
                    .map(|typ| rt.resolve_builtin_actor_type(typ).unwrap())
                    .collect();
                rt.validate_immediate_caller_type(&types)?;
            }
            _ => panic!("invalid branch passed to CallerValidation"),
        }
        Ok(())
    }

    // Creates an actor with the supplied CID and Address.
    pub fn create_actor(rt: &mut impl Runtime, args: Option<IpldBlock>) -> Result<(), ActorError> {
        let params: CreateActorArgs = decode_params!(args);

        rt.validate_immediate_caller_accept_any()?;
        // TODO Temporarily fine to use default as Undefined Cid, but may need to change in the future
        let actor_cid = if params.undef_cid { Cid::default() } else { params.cid };

        let actor_address = params.actor_id;

        rt.create_actor(actor_cid, actor_address, None)
    }

    /// Resolves address, and returns the resolved address (defaulting to 0 ID) and success boolean.
    pub fn resolve_address(
        rt: &mut impl Runtime,
        args: Option<IpldBlock>,
    ) -> Result<ResolveAddressResponse, ActorError> {
        let params: Address = decode_params!(args);

        rt.validate_immediate_caller_accept_any()?;
        let resolved = rt.resolve_address(&params);
        Ok(ResolveAddressResponse { id: resolved.unwrap_or(0), success: resolved.is_some() })
    }

    pub fn delete_actor(rt: &mut impl Runtime, args: Option<IpldBlock>) -> Result<(), ActorError> {
        let params: Address = decode_params!(args);

        rt.validate_immediate_caller_accept_any()?;
        rt.delete_actor(&params)
    }

    pub fn mutate_state(rt: &mut impl Runtime, args: Option<IpldBlock>) -> Result<(), ActorError> {
        let params: MutateStateArgs = decode_params!(args);

        rt.validate_immediate_caller_accept_any()?;

        match params.branch {
            x if x == MUTATE_IN_TRANSACTION => rt.transaction(|s: &mut State, _| {
                s.value = params.value;
                Ok(())
            }),

            _ => Err(actor_error!(illegal_argument; "Invalid mutate state command given" )),
        }
    }

    pub fn abort_with(_rt: &mut impl Runtime, args: Option<IpldBlock>) -> Result<(), ActorError> {
        let params: AbortWithArgs = decode_params!(args);

        if params.uncontrolled {
            panic!("Uncontrolled abort/error");
        }
        Err(ActorError::unchecked(params.code, params.message))
    }

    pub fn inspect_runtime(
        rt: &mut impl Runtime,
        _args: Option<IpldBlock>,
    ) -> Result<InspectRuntimeReturn, ActorError> {
        // TODO: NO_PARAMS
        rt.validate_immediate_caller_accept_any()?;
        Ok(InspectRuntimeReturn {
            caller: rt.message().caller(),
            receiver: rt.message().receiver(),
            value_received: rt.message().value_received(),
            curr_epoch: rt.curr_epoch(),
            current_balance: rt.current_balance(),
            state: rt.state()?,
        })
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
                Self::constructor(rt, args);
                Ok(RawBytes::default())
            }
            Some(Method::CallerValidation) => {
                Self::caller_validation(rt, args)?;
                Ok(RawBytes::default())
            }

            Some(Method::CreateActor) => {
                Self::create_actor(rt, args)?;
                Ok(RawBytes::default())
            }
            Some(Method::ResolveAddress) => {
                let res = Self::resolve_address(rt, args)?;
                Ok(RawBytes::serialize(res)?)
            }

            Some(Method::Send) => {
                let res: SendReturn = Self::send(rt, args)?;
                Ok(RawBytes::serialize(res)?)
            }

            Some(Method::DeleteActor) => {
                Self::delete_actor(rt, args)?;
                Ok(RawBytes::default())
            }

            Some(Method::MutateState) => {
                Self::mutate_state(rt, args)?;
                Ok(RawBytes::default())
            }

            Some(Method::AbortWith) => {
                Self::abort_with(rt, args)?;
                Ok(RawBytes::default())
            }

            Some(Method::InspectRuntime) => {
                let inspect = Self::inspect_runtime(rt, args)?;
                Ok(RawBytes::serialize(inspect)?)
            }

            None => Err(actor_error!(unhandled_message; "Invalid method")),
        }
    }
}
