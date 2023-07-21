use std::fmt::{Display, Formatter};

use fvm_ipld_encoding::de::DeserializeOwned;
use fvm_ipld_encoding::ipld_block::IpldBlock;
use fvm_shared::error::ExitCode;
use thiserror::Error;

/// The error type returned by actor method calls.
#[derive(Error, Debug, Clone, PartialEq, Eq)]
#[error("ActorError(exit_code: {exit_code}, msg: {msg})")]
pub struct ActorError {
    /// The exit code for this invocation.
    /// Codes less than `FIRST_USER_EXIT_CODE` are prohibited and will be overwritten by the VM.
    exit_code: ExitCode,
    /// Optional exit data
    data: Option<IpldBlock>,
    /// Message for debugging purposes,
    msg: String,
}

impl ActorError {
    /// Creates a new ActorError. This method does not check that the code is in the
    /// range of valid actor abort codes.
    pub fn unchecked(code: ExitCode, msg: String) -> Self {
        Self { exit_code: code, msg, data: None }
    }

    pub fn unchecked_with_data(code: ExitCode, msg: String, data: Option<IpldBlock>) -> Self {
        Self { exit_code: code, msg, data }
    }

    /// Creates a new ActorError. This method checks if the exit code is within the allowed range,
    /// and automatically converts it into a user code.
    pub fn checked(code: ExitCode, msg: String, data: Option<IpldBlock>) -> Self {
        let exit_code = match code {
            // This means the called actor did something wrong. We can't "make up" a
            // reasonable exit code.
            ExitCode::SYS_MISSING_RETURN
            | ExitCode::SYS_ILLEGAL_INSTRUCTION
            | ExitCode::SYS_ILLEGAL_EXIT_CODE => ExitCode::USR_UNSPECIFIED,
            // We don't expect any other system errors.
            code if code.is_system_error() => ExitCode::USR_ASSERTION_FAILED,
            // Otherwise, pass it through.
            code => code,
        };
        Self { exit_code, msg, data }
    }

    pub fn illegal_argument(msg: String) -> Self {
        Self { exit_code: ExitCode::USR_ILLEGAL_ARGUMENT, msg, data: None }
    }
    pub fn not_found(msg: String) -> Self {
        Self { exit_code: ExitCode::USR_NOT_FOUND, msg, data: None }
    }
    pub fn forbidden(msg: String) -> Self {
        Self { exit_code: ExitCode::USR_FORBIDDEN, msg, data: None }
    }
    pub fn insufficient_funds(msg: String) -> Self {
        Self { exit_code: ExitCode::USR_INSUFFICIENT_FUNDS, msg, data: None }
    }
    pub fn illegal_state(msg: String) -> Self {
        Self { exit_code: ExitCode::USR_ILLEGAL_STATE, msg, data: None }
    }
    pub fn serialization(msg: String) -> Self {
        Self { exit_code: ExitCode::USR_SERIALIZATION, msg, data: None }
    }
    pub fn unhandled_message(msg: String) -> Self {
        Self { exit_code: ExitCode::USR_UNHANDLED_MESSAGE, msg, data: None }
    }
    pub fn unspecified(msg: String) -> Self {
        Self { exit_code: ExitCode::USR_UNSPECIFIED, msg, data: None }
    }
    pub fn assertion_failed(msg: String) -> Self {
        Self { exit_code: ExitCode::USR_ASSERTION_FAILED, msg, data: None }
    }

    pub fn read_only(msg: String) -> Self {
        Self { exit_code: ExitCode::USR_READ_ONLY, msg, data: None }
    }

    /// Returns the exit code of the error.
    pub fn exit_code(&self) -> ExitCode {
        self.exit_code
    }

    /// Error message of the actor error.
    pub fn msg(&self) -> &str {
        &self.msg
    }

    /// Extracts the optional associated data without copying.
    pub fn take_data(&mut self) -> Option<IpldBlock> {
        std::mem::take(&mut self.data)
    }

    /// Prefix error message with a string message.
    pub fn wrap(mut self, msg: impl AsRef<str>) -> Self {
        self.msg = format!("{}: {}", msg.as_ref(), self.msg);
        self
    }
}

/// Converts a raw encoding error into a USR_SERIALIZATION.
impl From<fvm_ipld_encoding::Error> for ActorError {
    fn from(e: fvm_ipld_encoding::Error) -> Self {
        Self { exit_code: ExitCode::USR_SERIALIZATION, msg: e.to_string(), data: None }
    }
}

/// Converts an actor deletion error into a USR_ILLEGAL_ARGUMENT.
#[cfg(feature = "fil-actor")]
impl From<fvm_sdk::error::ActorDeleteError> for ActorError {
    fn from(e: fvm_sdk::error::ActorDeleteError) -> Self {
        Self { exit_code: ExitCode::USR_ILLEGAL_ARGUMENT, msg: e.to_string(), data: None }
    }
}

/// Converts a state-read error into an a USR_ILLEGAL_STATE.
#[cfg(feature = "fil-actor")]
impl From<fvm_sdk::error::StateReadError> for ActorError {
    fn from(e: fvm_sdk::error::StateReadError) -> Self {
        Self { exit_code: ExitCode::USR_ILLEGAL_STATE, data: None, msg: e.to_string() }
    }
}

/// Converts a state update error into an an actor error with the appropriate exit code.
#[cfg(feature = "fil-actor")]
impl From<fvm_sdk::error::StateUpdateError> for ActorError {
    fn from(e: fvm_sdk::error::StateUpdateError) -> Self {
        Self {
            exit_code: match e {
                fvm_sdk::error::StateUpdateError::ActorDeleted => ExitCode::USR_ILLEGAL_STATE,
                fvm_sdk::error::StateUpdateError::ReadOnly => ExitCode::USR_READ_ONLY,
            },
            data: None,
            msg: e.to_string(),
        }
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

// Convenience operations on a Result that may be an ActorError with exit code.
// Overriding the exit code is an explicit operation.
pub trait ActorResult<T> {
    // Wraps an error result with a context message.
    fn context<C>(self, context: C) -> Result<T, ActorError>
    where
        C: Display + 'static;

    // Wraps an error result with a lazily-evaluated context message.
    fn with_context<C, F>(self, f: F) -> Result<T, ActorError>
    where
        C: Display + 'static,
        F: FnOnce() -> C;

    // Maps an error result to a new exit code, preserving the message and data,
    // and wrapping with a context message.
    fn override_code<C>(self, code: ExitCode, context: C) -> Result<T, ActorError>
    where
        C: Display + 'static;

    // Maps an error result to a new exit code, preserving the message and data,
    // and wrapping with a lazily-evaluated context message.
    fn with_override_code<C, F>(self, code: ExitCode, f: F) -> Result<T, ActorError>
    where
        C: Display + 'static,
        F: FnOnce() -> C;
}

impl<T, E> ActorResult<T> for Result<T, E>
where
    E: Into<ActorError>,
{
    fn context<C>(self, context: C) -> Result<T, ActorError>
    where
        C: Display + 'static,
    {
        self.map_err(|err| err.into().wrap(context.to_string()))
    }

    fn with_context<C, F>(self, f: F) -> Result<T, ActorError>
    where
        C: Display + 'static,
        F: FnOnce() -> C,
    {
        self.map_err(|err| err.into().wrap(f().to_string()))
    }

    fn override_code<C>(self, code: ExitCode, context: C) -> Result<T, ActorError>
    where
        C: Display + 'static,
    {
        self.map_err(|err| {
            let err = err.into();
            ActorError::checked(
                code,
                format!("{}: {} (code {})", context, err.msg, err.exit_code),
                err.data,
            )
        })
    }

    fn with_override_code<C, F>(self, code: ExitCode, f: F) -> Result<T, ActorError>
    where
        C: Display + 'static,
        F: FnOnce() -> C,
    {
        self.map_err(|err| {
            let err = err.into();
            ActorError::checked(
                code,
                format!("{}: {} (code {})", f(), err.msg, err.exit_code),
                err.data,
            )
        })
    }
}

// FIXME rename to AsActorResult
// Adapts a result containing some error type into an actor error.
// Implementations of this trait on types that might wrap an ActorError should
// extract and propagate the wrapped code in preference to the default code provided as an argument.
// By design there is *no implementation for ActorError* or any trait that it implements
// (e.g. Error). Code which has an ActorError should use the methods of ActorResult.
// Implementations must be on concrete error types to avoid potentially conflicting implementations
// for an error type that could implement multiple traits.
pub trait AsActorError<T>: Sized {
    // Converts error result into an ActorError with a context message.
    // If the receiver wraps an ActorError, the exit code must be propagated,
    // otherwise the code provided as an argument is used.
    fn context_code<C>(self, code: ExitCode, context: C) -> Result<T, ActorError>
    where
        C: Display + 'static;

    // Converts error result into an ActorError with a lazily-evaluated context message.
    // If the receiver wraps an ActorError, the exit code must be propagated,
    // otherwise the code provided as an argument is used.
    fn with_context_code<C, F>(self, code: ExitCode, f: F) -> Result<T, ActorError>
    where
        C: Display + 'static,
        F: FnOnce() -> C;
}

impl<T> AsActorError<T> for Result<T, anyhow::Error> {
    fn context_code<C>(self, code: ExitCode, context: C) -> Result<T, ActorError>
    where
        C: Display + 'static,
    {
        context_code(self, code, context)
    }

    fn with_context_code<C, F>(self, code: ExitCode, f: F) -> Result<T, ActorError>
    where
        C: Display + 'static,
        F: FnOnce() -> C,
    {
        with_context_code(self, code, f)
    }
}

impl<T> AsActorError<T> for Result<T, fvm_ipld_amt::Error> {
    fn context_code<C>(self, code: ExitCode, context: C) -> Result<T, ActorError>
    where
        C: Display + 'static,
    {
        context_code(self, code, context)
    }

    fn with_context_code<C, F>(self, code: ExitCode, f: F) -> Result<T, ActorError>
    where
        C: Display + 'static,
        F: FnOnce() -> C,
    {
        with_context_code(self, code, f)
    }
}

impl<T> AsActorError<T> for Result<T, fvm_ipld_hamt::Error> {
    fn context_code<C>(self, code: ExitCode, context: C) -> Result<T, ActorError>
    where
        C: Display + 'static,
    {
        context_code(self, code, context)
    }

    fn with_context_code<C, F>(self, code: ExitCode, f: F) -> Result<T, ActorError>
    where
        C: Display + 'static,
        F: FnOnce() -> C,
    {
        with_context_code(self, code, f)
    }
}

// FIXME what about EVM KAMT?
// - defined in another module, not imported here (why is HAMT imported here anyway?)
// - can wrap an ActorError
// We can't implement this trait either here or where Kamt::Error is used.

// Basic implementation for opaque errors, that are known not to be hiding an exit code.
impl<T, E: Into<OpaqueError>> AsActorError<T> for Result<T, E> {
    fn context_code<C>(self, code: ExitCode, context: C) -> Result<T, ActorError>
    where
        C: Display + 'static,
    {
        self.map_err(|err| ActorError::checked(code, format!("{}: {}", context, err.into()), None))
    }

    fn with_context_code<C, F>(self, code: ExitCode, f: F) -> Result<T, ActorError>
    where
        C: Display + 'static,
        F: FnOnce() -> C,
    {
        self.map_err(|err| ActorError::checked(code, format!("{}: {}", f(), err.into()), None))
    }
}

// Supports conversion of Options to Results with ActorErrors.
impl<T> AsActorError<T> for Option<T> {
    fn context_code<C>(self, code: ExitCode, context: C) -> Result<T, ActorError>
    where
        C: Display + 'static,
    {
        self.ok_or_else(|| ActorError { exit_code: code, msg: context.to_string(), data: None })
    }

    fn with_context_code<C, F>(self, code: ExitCode, f: F) -> Result<T, ActorError>
    where
        C: Display + 'static,
        F: FnOnce() -> C,
    {
        self.ok_or_else(|| ActorError { exit_code: code, msg: f().to_string(), data: None })
    }
}

pub fn deserialize_block<T>(ret: Option<IpldBlock>) -> Result<T, ActorError>
where
    T: DeserializeOwned,
{
    ret.context_code(ExitCode::USR_ASSERTION_FAILED, "return expected".to_string())?
        .deserialize()
        .context_code(ExitCode::USR_SERIALIZATION, "deserialization failed")
}

// Wrapper for an error that is opaque, emerging from low level dependencies before an
// appropriate exit code has been determined.
// By design, there is *no conversion from ActorError* or any Error type that might wrap
// an ActorError. This prevents unintentional overriding of the contained exit code.
// A struct must be used here (rather than a trait) in order to prevent conflicting implementations
// of AsActorError for a type that might possibly implement multiple traits.
pub struct OpaqueError(String);

impl OpaqueError {
    pub fn new(msg: impl Into<String>) -> Self {
        Self(msg.into())
    }
}

impl Display for OpaqueError {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl From<String> for OpaqueError {
    fn from(val: String) -> Self {
        OpaqueError::new(val)
    }
}

impl From<fvm_ipld_encoding::Error> for OpaqueError {
    fn from(val: fvm_ipld_encoding::Error) -> Self {
        OpaqueError::new(val.to_string())
    }
}

impl From<unsigned_varint::decode::Error> for OpaqueError {
    fn from(val: unsigned_varint::decode::Error) -> Self {
        OpaqueError::new(val.to_string())
    }
}

// Extension method to extract any ActorError wrapped in another error type.
// Implementations should recursively downcast any wrapped error types that may themselves
// wrap an ActorError.
trait ActorErrorDowncast {
    // Returns an ActorError if the receiver is wrapping one,
    // else formats the receiver as a string.
    fn to_actor_error(self) -> Result<ActorError, String>;
}

// Extracts any ActorError wrapped as an Anyhow error.
impl ActorErrorDowncast for anyhow::Error {
    fn to_actor_error(self) -> Result<ActorError, String> {
        let e = self;
        let e = match e.downcast::<ActorError>() {
            Ok(inner) => return Ok(inner),
            Err(e) => e,
        };
        // Handle all other specific error types that could wrap ActorError.
        let e = match e.downcast::<fvm_ipld_amt::Error>() {
            Ok(inner) => {
                return match inner.to_actor_error() {
                    Ok(actor) => Ok(actor),
                    Err(e) => Err(e),
                }
            }
            Err(e) => e,
        };
        let e = match e.downcast::<fvm_ipld_hamt::Error>() {
            Ok(inner) => {
                return match inner.to_actor_error() {
                    Ok(actor) => Ok(actor),
                    Err(e) => Err(e),
                }
            }
            Err(e) => e,
        };
        Err(e.to_string())
    }
}

// Extracts any ActorError wrapped in an AMT error.
impl ActorErrorDowncast for fvm_ipld_amt::Error {
    fn to_actor_error(self) -> Result<ActorError, String> {
        match self {
            fvm_ipld_amt::Error::Dynamic(e) => match e.to_actor_error() {
                Ok(actor) => Ok(actor),
                Err(e) => Err(e),
            },
            _ => Err(self.to_string()),
        }
    }
}

// Extracts any ActorError wrapped in an HAMT error.
impl ActorErrorDowncast for fvm_ipld_hamt::Error {
    fn to_actor_error(self) -> Result<ActorError, String> {
        match self {
            fvm_ipld_hamt::Error::Dynamic(e) => match e.to_actor_error() {
                Ok(actor) => Ok(actor),
                Err(e) => Err(e),
            },
            _ => Err(self.to_string()),
        }
    }
}

// Shared implementation of context_code and with_context_code for types that
// might wrap an ActorError. These propagate any underlying ActorError's code and data.
fn context_code<T, E, C>(r: Result<T, E>, code: ExitCode, context: C) -> Result<T, ActorError>
where
    E: ActorErrorDowncast + Display,
    C: Display + 'static,
{
    r.map_err(|err| match err.to_actor_error() {
        Ok(e) => e.wrap(context.to_string()),
        Err(e) => ActorError::checked(code, format!("{}: {}", context, e), None),
    })
}

fn with_context_code<T, E, C, F>(r: Result<T, E>, code: ExitCode, f: F) -> Result<T, ActorError>
where
    E: ActorErrorDowncast + Display,
    C: Display + 'static,
    F: FnOnce() -> C,
{
    r.map_err(|err| match err.to_actor_error() {
        Ok(e) => e.wrap(f().to_string()),
        Err(e) => ActorError::checked(code, format!("{}: {}", f(), e), None),
    })
}

#[cfg(test)]
mod test {
    use crate::{ActorError, ActorResult, AsActorError};
    use anyhow::anyhow;
    use fvm_ipld_encoding::ipld_block::IpldBlock;
    use fvm_shared::error::ExitCode;
    use std::fmt::Display;

    #[test]
    fn context_propagates_code() {
        let block = IpldBlock::serialize_cbor(&1).unwrap();
        let root_code = ExitCode::USR_UNSPECIFIED;
        let wrap_code = ExitCode::USR_ILLEGAL_STATE;
        let actor_error = ActorError::checked(root_code, "123".to_string(), block);
        // Methods from ActorResult on Result<(), ActorError>
        verify(root_code, "123", Err(actor_error.clone()));
        verify(root_code, "456: 123", Err(actor_error.clone()).context("456"));
        // Code can be explicitly overridden (the original one is preserved in the message).
        verify(
            wrap_code,
            format!("456: 123 (code {})", root_code).as_str(),
            Err(actor_error.clone()).override_code(wrap_code, "456"),
        );

        // Methods from AsActorResult that adapt into Result<(), ActorError>.
        // The inner error has no exit code so the wrapping one is used.
        verify(wrap_code, "456: 123", Err(anyhow!("123")).context_code(wrap_code, "456"));
        verify(
            wrap_code,
            "456: Vector length does not match bitmap",
            Err(fvm_ipld_amt::Error::InvalidVecLength).context_code(wrap_code, "456"),
        );

        // Methods from AsActorResult on values that *are* wrapping an underlying ActorError.
        // The underlying code should propagate instead of the wrapping one.
        // ActorError wrapped in anyhow::Error
        verify(root_code, "123", Err::<(), anyhow::Error>(actor_error.clone().into()));
        verify(
            root_code, // The underlying code prevails
            "456: 123",
            Err::<(), anyhow::Error>(actor_error.clone().into()).context_code(wrap_code, "456"),
        );
        verify(
            wrap_code, // But can be overridden explicitly
            format!("789: 456: 123 (code {})", root_code).as_str(),
            Err::<(), anyhow::Error>(actor_error.clone().into())
                .context_code(wrap_code, "456")
                .override_code(wrap_code, "789"),
        );

        // ActorError wrapped in fvm_ipld_amt::Error::Dynamic(anyhow::Error())
        verify(root_code, "123", Err(fvm_ipld_amt::Error::Dynamic(actor_error.clone().into())));
        verify(
            root_code, // The underlying code prevails
            "456: 123",
            Err(fvm_ipld_amt::Error::Dynamic(actor_error.clone().into()))
                .context_code(wrap_code, "456"),
        );

        // Deep nesting
        verify(
            root_code, // The underlying code prevails
            "123",
            Err(fvm_ipld_amt::Error::Dynamic(anyhow!(fvm_ipld_hamt::Error::Dynamic(
                actor_error.clone().into()
            )))),
        );
        verify(
            root_code,
            "456: 123",
            Err(fvm_ipld_amt::Error::Dynamic(anyhow!(fvm_ipld_hamt::Error::Dynamic(
                actor_error.into()
            ))))
            .context_code(wrap_code, "456"),
        );
    }

    fn verify<E: Display>(code: ExitCode, msg: &str, result: Result<(), E>) {
        assert_eq!(
            format!("ActorError(exit_code: {}, msg: {})", code, msg),
            result.unwrap_err().to_string()
        );
    }
}
