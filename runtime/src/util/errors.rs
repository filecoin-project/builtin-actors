use crate::{ActorError, AsActorError};
use fvm_shared::error::ExitCode;
use std::fmt::Display;

// Convenient wrappers for error conversion.
pub trait AsActorErrors<T>: Sized {
    fn or_illegal_state<C>(self, context: C) -> Result<T, ActorError>
    where
        C: Display + 'static;

    fn or_with_illegal_state<C, F>(self, f: F) -> Result<T, ActorError>
    where
        C: Display + 'static,
        F: FnOnce() -> C;

    fn or_assertion_failed<C>(self, context: C) -> Result<T, ActorError>
    where
        C: Display + 'static;

    fn or_with_assertion_failed<C, F>(self, f: F) -> Result<T, ActorError>
    where
        C: Display + 'static,
        F: FnOnce() -> C;
}

// Note: E should be std::error::Error, revert to this after anyhow:Error is no longer used.
impl<T, E: Display> AsActorErrors<T> for Result<T, E> {
    fn or_illegal_state<C>(self, context: C) -> Result<T, ActorError>
    where
        C: Display + 'static,
    {
        self.context_code(ExitCode::USR_ILLEGAL_STATE, context)
    }

    fn or_with_illegal_state<C, F>(self, f: F) -> Result<T, ActorError>
    where
        C: Display + 'static,
        F: FnOnce() -> C,
    {
        self.with_context_code(ExitCode::USR_ILLEGAL_STATE, f)
    }

    fn or_assertion_failed<C>(self, context: C) -> Result<T, ActorError>
    where
        C: Display + 'static,
    {
        self.context_code(ExitCode::USR_ASSERTION_FAILED, context)
    }

    fn or_with_assertion_failed<C, F>(self, f: F) -> Result<T, ActorError>
    where
        C: Display + 'static,
        F: FnOnce() -> C,
    {
        self.with_context_code(ExitCode::USR_ASSERTION_FAILED, f)
    }
}
