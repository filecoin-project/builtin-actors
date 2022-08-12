use {
    bytes::Bytes,
    fvm_ipld_encoding::Cbor,
    serde::{Deserialize, Serialize},
    std::fmt::Debug,
    strum_macros::Display,
};

/// Output of EVM execution.
#[derive(Clone, PartialEq)]
pub struct Output {
    /// EVM exited with this status code.
    pub status_code: StatusCode,
    /// How much gas was left after execution
    pub gas_left: i64,
    /// Output data returned.
    pub output_data: Bytes,
    // indicates if revert was requested
    pub reverted: bool,
}

impl Debug for Output {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Output")
            .field("status_code", &self.status_code)
            .field("gas_left", &self.gas_left)
            .field("output_data", &hex::encode(&self.output_data))
            .field("reverted", &self.reverted)
            .finish()
    }
}


/// Message status code.
#[must_use]
#[derive(Clone, Debug, Display, PartialEq, Serialize, Deserialize)]
pub enum StatusCode {
    /// Execution finished with success.
    #[strum(serialize = "success")]
    Success,

    /// Generic execution failure.
    #[strum(serialize = "failure")]
    Failure,

    /// Execution terminated with REVERT opcode.
    #[strum(serialize = "revert")]
    Revert,

    /// The execution has run out of gas.
    #[strum(serialize = "out of gas")]
    OutOfGas,

    /// The designated INVALID instruction has been hit during execution.
    ///
    /// [EIP-141](https://github.com/ethereum/EIPs/blob/master/EIPS/eip-141.md)
    /// defines the instruction 0xfe as INVALID instruction to indicate execution
    /// abortion coming from high-level languages. This status code is reported
    /// in case this INVALID instruction has been encountered.
    #[strum(serialize = "invalid instruction")]
    InvalidInstruction,

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
    ArgumentOutOfRange,

    /// The caller does not have enough funds for value transfer.
    #[strum(serialize = "insufficient balance")]
    InsufficientBalance,

    /// EVM implementation generic internal error.
    #[strum(serialize = "internal error")]
    InternalError(String),
}

impl Cbor for StatusCode {}
