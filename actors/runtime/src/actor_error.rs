use std::{fmt::Display, num::TryFromIntError};

use fvm_shared::error::ExitCode;
use thiserror::Error;

/// The error type returned by actor method calls.
#[derive(Error, Debug, Clone, PartialEq)]
#[error("ActorError(exit_code: {exit_code:?}, msg: {msg})")]
pub struct ActorError {
    /// The exit code for this invocation.
    /// Codes less than `FIRST_ACTOR_EXIT_CODE` are prohibited and will be overwritten by the VM.
    exit_code: ExitCode,
    /// Message for debugging purposes,
    msg: String,
}

impl ActorError {
    /// Creates a new ActorError. This method does not check that the code is in the
    /// range of valid actor abort codes.
    pub fn unchecked(code: ExitCode, msg: String) -> Self {
        Self { exit_code: code, msg }
    }

    pub fn illegal_argument(msg: String) -> Self {
        Self { exit_code: ExitCode::USR_ILLEGAL_ARGUMENT, msg }
    }
    pub fn not_found(msg: String) -> Self {
        Self { exit_code: ExitCode::USR_NOT_FOUND, msg }
    }
    pub fn forbidden(msg: String) -> Self {
        Self { exit_code: ExitCode::USR_FORBIDDEN, msg }
    }
    pub fn insufficient_funds(msg: String) -> Self {
        Self { exit_code: ExitCode::USR_INSUFFICIENT_FUNDS, msg }
    }
    pub fn illegal_state(msg: String) -> Self {
        Self { exit_code: ExitCode::USR_ILLEGAL_STATE, msg }
    }
    pub fn serialization(msg: String) -> Self {
        Self { exit_code: ExitCode::USR_SERIALIZATION, msg }
    }
    pub fn unhandled_message(msg: String) -> Self {
        Self { exit_code: ExitCode::USR_UNHANDLED_MESSAGE, msg }
    }
    pub fn unspecified(msg: String) -> Self {
        Self { exit_code: ExitCode::USR_UNSPECIFIED, msg }
    }
    pub fn user_assertion_failed(msg: String) -> Self {
        Self { exit_code: ExitCode::USR_ASSERTION_FAILED, msg }
    }

    /// Returns the exit code of the error.
    pub fn exit_code(&self) -> ExitCode {
        self.exit_code
    }

    /// Error message of the actor error.
    pub fn msg(&self) -> &str {
        &self.msg
    }

    /// Prefix error message with a string message.
    pub fn wrap(mut self, msg: impl AsRef<str>) -> Self {
        self.msg = format!("{}: {}", msg.as_ref(), self.msg);
        self
    }
}

/// Converts a raw encoding error into an ErrSerialization.
impl From<fvm_ipld_encoding::Error> for ActorError {
    fn from(e: fvm_ipld_encoding::Error) -> Self {
        Self { exit_code: ExitCode::USR_SERIALIZATION, msg: e.to_string() }
    }
}

impl<E: std::error::Error> From<fvm_ipld_amt::Error<E>> for ActorError {
    fn from(e: fvm_ipld_amt::Error<E>) -> Self {
        Self { exit_code: ExitCode::USR_SERIALIZATION, msg: e.to_string() }
    }
}

impl<E: std::error::Error> From<fvm_ipld_hamt::Error<E>> for ActorError {
    fn from(e: fvm_ipld_hamt::Error<E>) -> Self {
        Self { exit_code: ExitCode::USR_SERIALIZATION, msg: e.to_string() }
    }
}

impl<E: std::error::Error> From<fvm_ipld_encoding::CborStoreError<E>> for ActorError {
    fn from(e: fvm_ipld_encoding::CborStoreError<E>) -> Self {
        Self { exit_code: ExitCode::USR_ILLEGAL_STATE, msg: e.to_string() }
    }
}

impl From<fvm_ipld_bitfield::Error> for ActorError {
    fn from(e: fvm_ipld_bitfield::Error) -> Self {
        // TODO: correct code?
        Self { exit_code: ExitCode::USR_ILLEGAL_STATE, msg: e.to_string() }
    }
}

impl From<TryFromIntError> for ActorError {
    fn from(e: TryFromIntError) -> Self {
        // TODO: correct code?
        Self { exit_code: ExitCode::USR_SERIALIZATION, msg: e.to_string() }
    }
}

impl From<fvm_ipld_bitfield::OutOfRangeError> for ActorError {
    fn from(e: fvm_ipld_bitfield::OutOfRangeError) -> Self {
        // TODO: correct code?
        Self { exit_code: ExitCode::USR_SERIALIZATION, msg: e.to_string() }
    }
}

impl<E: std::error::Error> From<crate::util::MultiMapError<E>> for ActorError {
    fn from(e: crate::util::MultiMapError<E>) -> Self {
        match e {
            crate::util::MultiMapError::Amt(e) => e.into(),
            crate::util::MultiMapError::Hamt(e) => e.into(),
        }
    }
}

impl<U: Into<ActorError>, E: std::error::Error> From<crate::util::MultiMapEitherError<U, E>>
    for ActorError
{
    fn from(e: crate::util::MultiMapEitherError<U, E>) -> Self {
        match e {
            crate::util::MultiMapEitherError::User(e) => e.into(),
            crate::util::MultiMapEitherError::MultiMap(e) => e.into(),
        }
    }
}

impl<U: Into<ActorError>, E: std::error::Error> From<fvm_ipld_amt::EitherError<U, E>>
    for ActorError
{
    fn from(e: fvm_ipld_amt::EitherError<U, E>) -> Self {
        match e {
            fvm_ipld_amt::EitherError::User(e) => e.into(),
            fvm_ipld_amt::EitherError::Amt(e) => e.into(),
        }
    }
}

impl<U: Into<ActorError>, E: std::error::Error> From<fvm_ipld_hamt::EitherError<U, E>>
    for ActorError
{
    fn from(e: fvm_ipld_hamt::EitherError<U, E>) -> Self {
        match e {
            fvm_ipld_hamt::EitherError::User(e) => e.into(),
            fvm_ipld_hamt::EitherError::Hamt(e) => e.into(),
        }
    }
}

/// Converts an actor deletion error into an actor error with the appropriate exit code. This
/// facilitates propagation.
#[cfg(feature = "fil-actor")]
impl From<fvm_sdk::error::ActorDeleteError> for ActorError {
    fn from(e: fvm_sdk::error::ActorDeleteError) -> Self {
        Self { exit_code: ExitCode::USR_ILLEGAL_ARGUMENT, msg: e.to_string() }
    }
}

/// Converts a no-state error into an an actor error with the appropriate exit code (illegal actor).
/// This facilitates propagation.
#[cfg(feature = "fil-actor")]
impl From<fvm_sdk::error::NoStateError> for ActorError {
    fn from(e: fvm_sdk::error::NoStateError) -> Self {
        Self { exit_code: ExitCode::USR_ILLEGAL_STATE, msg: e.to_string() }
    }
}

/// Convenience macro for generating Actor Errors
#[macro_export]
macro_rules! actor_error {
    // Error with only one stringable expression
    ( $code:ident; $msg:expr ) => { $crate::ActorError::$code($msg.to_string()) };

    // String with positional arguments
    ( $code:ident; $msg:literal $(, $ex:expr)+ ) => {
        $crate::ActorError::$code(format!($msg, $($ex,)*))
    };

    // Error with only one stringable expression, with comma separator
    ( $code:ident, $msg:expr ) => { $crate::actor_error!($code; $msg) };

    // String with positional arguments, with comma separator
    ( $code:ident, $msg:literal $(, $ex:expr)+ ) => {
        $crate::actor_error!($code; $msg $(, $ex)*)
    };
}

pub trait ActorContext<T> {
    fn context<C>(self, context: C) -> Result<T, ActorError>
    where
        C: Display + Send + Sync + 'static;
    fn with_context<C, F>(self, f: F) -> Result<T, ActorError>
    where
        C: Display + Send + Sync + 'static,
        F: FnOnce() -> C;
}

impl<T, E: Into<ActorError>> ActorContext<T> for Result<T, E> {
    fn context<C>(self, context: C) -> Result<T, ActorError>
    where
        C: Display + Send + Sync + 'static,
    {
        self.map_err(|err| {
            let mut err: ActorError = err.into();
            err.msg = format!("{}: {}", context, err.msg);
            err
        })
    }

    fn with_context<C, F>(self, f: F) -> Result<T, ActorError>
    where
        C: Display + Send + Sync + 'static,
        F: FnOnce() -> C,
    {
        self.map_err(|err| {
            let mut err: ActorError = err.into();
            err.msg = format!("{}: {}", f(), err.msg);
            err
        })
    }
}

// TODO: remove once the runtime doesn't use anyhow::Result anymore
impl From<anyhow::Error> for ActorError {
    fn from(e: anyhow::Error) -> Self {
        match e.downcast::<ActorError>() {
            Ok(actor_err) => actor_err,
            Err(other) => ActorError::unchecked(
                ExitCode::USR_ILLEGAL_ARGUMENT,
                format!("runtime error: {}", other),
            ),
        }
    }
}
