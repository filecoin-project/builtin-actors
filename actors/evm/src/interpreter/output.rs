use fil_actors_runtime::ActorError as RTActorError;
use {bytes::Bytes, std::fmt::Debug, strum_macros::Display};

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum Outcome {
    #[default]
    Return,
    Revert,
}

/// Output of EVM execution.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Output {
    /// Indicates the "outcome" of the execution.
    pub outcome: Outcome,
    /// The return data.
    pub return_data: Bytes,
}

/// Message status code.
#[must_use]
#[derive(Clone, Debug, Display, PartialEq, Eq)]
pub enum StatusCode {
    /// The designated INVALID instruction has been hit during execution.
    ///
    /// [EIP-141](https://github.com/ethereum/EIPs/blob/master/EIPS/eip-141.md)
    /// defines the instruction 0xfe as INVALID instruction to indicate execution
    /// abortion coming from high-level languages. This status code is reported
    /// in case this INVALID instruction has been encountered.
    #[strum(serialize = "invalid instruction")]
    InvalidInstruction,

    /// An argument passed to an instruction does not meet expectations.
    #[strum(serialize = "invalid argument")]
    InvalidArgument(String),

    /// An undefined instruction has been encountered.
    #[strum(serialize = "undefined instruction")]
    UndefinedInstruction,

    /// The execution has attempted to put more items on the EVM stack
    /// than the specified limit.
    #[strum(serialize = "stack overflow")]
    StackOverflow,

    /// Execution of an opcode has required more items on the EVM stack.
    #[strum(serialize = "stack underflow")]
    StackUnderflow,

    /// Execution has violated the jump destination restrictions.
    #[strum(serialize = "bad jump destination")]
    BadJumpDestination,

    /// Tried to read outside memory bounds.
    ///
    /// An example is RETURNDATACOPY reading past the available buffer.
    #[strum(serialize = "invalid memory access")]
    InvalidMemoryAccess,

    /// Call depth has exceeded the limit (if any)
    #[strum(serialize = "call depth exceeded")]
    CallDepthExceeded,

    /// Tried to execute an operation which is restricted in static mode.
    #[strum(serialize = "static mode violation")]
    StaticModeViolation,

    /// A call to a precompiled or system contract has ended with a failure.
    ///
    /// An example: elliptic curve functions handed invalid EC points.
    #[strum(serialize = "precompile failure")]
    PrecompileFailure,

    /// Contract validation has failed.
    #[strum(serialize = "contract validation failure")]
    ContractValidationFailure,

    /// An argument to a state accessing method has a value outside of the
    /// accepted range of values.
    #[strum(serialize = "argument out of range")]
    ArgumentOutOfRange(String),

    /// The caller does not have enough funds for value transfer.
    #[strum(serialize = "insufficient balance")]
    InsufficientBalance,

    /// EVM implementation generic internal error.
    #[strum(serialize = "internal error")]
    InternalError(String),

    /// Invalid Address
    #[strum(serialize = "bad address")]
    BadAddress(String),

    /// Nested Actor invocation Error
    #[strum(serialize = "runtime actor error")]
    ActorError(RTActorError),
}

// Map ActorError to a generic internal error status code.
impl From<RTActorError> for StatusCode {
    fn from(ae: RTActorError) -> Self {
        Self::ActorError(ae)
    }
}

impl From<fvm_ipld_encoding::Error> for StatusCode {
    fn from(err: fvm_ipld_encoding::Error) -> Self {
        Self::InternalError(format!("IPLD error: {:?}", &err))
    }
}
