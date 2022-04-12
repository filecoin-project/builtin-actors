// Copyright 2019-2022 ChainSafe Systems
// SPDX-License-Identifier: Apache-2.0, MIT

use anyhow::anyhow;
use fvm_ipld_amt::Error as AmtError;
use fvm_ipld_hamt::Error as HamtError;
use fvm_shared::error::ExitCode;

use crate::ActorError;

/// Trait to allow multiple error types to be able to be downcasted into an `ActorError`.
pub trait ActorDowncast {
    /// Downcast a dynamic std Error into an `ActorError`. If the error cannot be downcasted
    /// into an ActorError automatically, use the provided `ExitCode` to generate a new error.
    fn downcast_default(self, default_exit_code: ExitCode, msg: impl AsRef<str>) -> ActorError;

    /// Wrap the error with a message, without overwriting an exit code.
    fn downcast_wrap(self, msg: impl AsRef<str>) -> anyhow::Error;
}

impl ActorDowncast for anyhow::Error {
    fn downcast_default(self, default_exit_code: ExitCode, msg: impl AsRef<str>) -> ActorError {
        match downcast_util(self) {
            Ok(actor_error) => actor_error.wrap(msg),
            Err(other) => {
                ActorError::unchecked(default_exit_code, format!("{}: {}", msg.as_ref(), other))
            }
        }
    }
    fn downcast_wrap(self, msg: impl AsRef<str>) -> anyhow::Error {
        match downcast_util(self) {
            Ok(actor_error) => anyhow!(actor_error.wrap(msg)),
            Err(other) => anyhow!("{}: {}", msg.as_ref(), other),
        }
    }
}

impl<E: std::error::Error> ActorDowncast for AmtError<E> {
    fn downcast_default(self, default_exit_code: ExitCode, msg: impl AsRef<str>) -> ActorError {
        match self {
            // AmtError::Dynamic(e) => e.downcast_default(default_exit_code, msg),
            // todo: proper downcast
            other => {
                ActorError::unchecked(default_exit_code, format!("{}: {}", msg.as_ref(), other))
            }
        }
    }
    fn downcast_wrap(self, msg: impl AsRef<str>) -> anyhow::Error {
        match self {
            // AmtError::Dynamic(e) => e.downcast_wrap(msg),
            // todo: proper downcast
            other => anyhow!("{}: {}", msg.as_ref(), other),
        }
    }
}

impl<E: std::error::Error> ActorDowncast for HamtError<E> {
    fn downcast_default(self, default_exit_code: ExitCode, msg: impl AsRef<str>) -> ActorError {
        match self {
            // HamtError::Dynamic(e) => e.downcast_default(default_exit_code, msg),
            // todo: proper downcast
            other => {
                ActorError::unchecked(default_exit_code, format!("{}: {}", msg.as_ref(), other))
            }
        }
    }
    fn downcast_wrap(self, msg: impl AsRef<str>) -> anyhow::Error {
        match self {
            // HamtError::Dynamic(e) => e.downcast_wrap(msg),
            // todo: proper downcast
            other => anyhow!("{}: {}", msg.as_ref(), other),
        }
    }
}

/// Attempts to downcast a `Box<dyn std::error::Error>` into an actor error.
/// Returns `Ok` with the actor error if it can be downcasted automatically
/// and returns `Err` with the original error if it cannot.
fn downcast_util(error: anyhow::Error) -> anyhow::Result<ActorError> {
    // Check if error is ActorError, return as such
    let error = match error.downcast::<ActorError>() {
        Ok(actor_err) => return Ok(actor_err),
        Err(other) => other,
    };

    // Could not be downcasted automatically to actor error, return initial dynamic error.
    Err(error)
}
