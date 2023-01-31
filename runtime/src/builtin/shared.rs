// Copyright 2019-2022 ChainSafe Systems
// SPDX-License-Identifier: Apache-2.0, MIT

use crate::runtime::builtins::Type;
use crate::{actor_error, ActorContext, ActorError};
use fvm_ipld_encoding::ipld_block::IpldBlock;
use fvm_shared::address::Address;
use fvm_shared::METHOD_SEND;
use fvm_shared::{ActorID, MethodNum};

use crate::runtime::Runtime;

pub const HAMT_BIT_WIDTH: u32 = 5;

pub const FIRST_ACTOR_SPECIFIC_EXIT_CODE: u32 = 32;

/// ResolveToActorID resolves the given address to its actor ID.
/// If an actor ID for the given address doesn't exist yet, it tries to create one by sending
/// a zero balance to the given address.
pub fn resolve_to_actor_id(
    rt: &mut impl Runtime,
    address: &Address,
) -> Result<ActorID, ActorError> {
    // if we are able to resolve it to an ID address, return the resolved address
    if let Some(id) = rt.resolve_address(address) {
        return Ok(id);
    }

    // send 0 balance to the account so an ID address for it is created and then try to resolve
    extract_send_result(rt.send_simple(
        address,
        METHOD_SEND,
        Default::default(),
        Default::default(),
    ))
    .with_context(|| format!("failed to send zero balance to address {}", address))?;

    if let Some(id) = rt.resolve_address(address) {
        return Ok(id);
    }

    Err(actor_error!(illegal_argument, "failed to resolve or initialize address {}", address))
}

// The lowest FRC-42 method number.
pub const FIRST_EXPORTED_METHOD_NUMBER: MethodNum = 1 << 24;

// Checks whether the caller is allowed to invoke some method number.
// All method numbers below the FRC-42 range are restricted to built-in actors
// (including the account and multisig actors).
// Methods may subsequently enforce tighter restrictions.
pub fn restrict_internal_api<RT>(rt: &mut RT, method: MethodNum) -> Result<(), ActorError>
where
    RT: Runtime,
{
    if method >= FIRST_EXPORTED_METHOD_NUMBER {
        return Ok(());
    }
    let caller = rt.message().caller();
    let code_cid = rt.get_actor_code_cid(&caller.id().unwrap());
    match code_cid {
        None => {
            return Err(
                actor_error!(forbidden; "no code for caller {} of method {}", caller, method),
            );
        }
        Some(code_cid) => {
            let builtin_type = rt.resolve_builtin_actor_type(&code_cid);
            match builtin_type {
                None | Some(Type::EVM) => {
                    return Err(
                        actor_error!(forbidden; "caller {} of method {} must be built-in", caller, method),
                    );
                }

                // Anything else is a valid built-in caller of the internal API
                Some(_) => {}
            }
        }
    }
    Ok(())
}

pub fn extract_send_result(
    res: Result<fvm_shared::Response, fvm_shared::error::ErrorNumber>,
) -> Result<Option<IpldBlock>, ActorError> {
    match res {
        Ok(ret) => {
            if ret.exit_code.is_success() {
                Ok(ret.return_data)
            } else {
                Err(ActorError::checked(
                    ret.exit_code,
                    format!("send aborted with code {}", ret.exit_code),
                    ret.return_data,
                ))
            }
        }
        Err(err) => Err(match err {
            // Some of these errors are from operations in the Runtime or SDK layer
            // before or after the underlying VM send syscall.
            fvm_shared::error::ErrorNumber::NotFound => {
                // This means that the receiving actor doesn't exist.
                actor_error!(unspecified; "receiver not found")
            }
            fvm_shared::error::ErrorNumber::InsufficientFunds => {
                // This means that the send failed because we have insufficient funds. We will
                // get a _syscall error_, not an exit code, because the target actor will not
                // run (and therefore will not exit).
                actor_error!(insufficient_funds; "not enough funds")
            }
            fvm_shared::error::ErrorNumber::LimitExceeded => {
                // This means we've exceeded the recursion limit.
                actor_error!(assertion_failed; "recursion limit exceeded")
            }
            fvm_shared::error::ErrorNumber::ReadOnly => ActorError::unchecked(
                fvm_shared::error::ExitCode::USR_READ_ONLY,
                "attempted to mutate state while in readonly mode".into(),
            ),
            err => {
                // We don't expect any other syscall exit codes.
                actor_error!(assertion_failed; "unexpected error: {}", err)
            }
        }),
    }
}
