pub mod address;
pub mod bytecode;
mod execution;
mod instructions;
mod memory;
mod output;
mod precompiles;
mod stack;
mod system;
mod uints;

#[cfg(test)]
pub mod test_util;

pub use {
    bytecode::Bytecode,
    execution::{execute, opcodes, ExecutionState},
    output::{Outcome, Output},
    system::System,
    uints::{U256, U512},
};

/// The kind of call-like instruction.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CallKind {
    Call,
    DelegateCall,
    StaticCall,
}
