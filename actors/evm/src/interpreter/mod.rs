pub mod address;
pub mod bytecode;
pub mod execution;
pub mod instructions;
pub mod memory;
pub mod output;
pub mod precompiles;
pub mod stack;
pub mod system;
pub mod uints;

pub use {
    bytecode::Bytecode,
    execution::{execute, ExecutionState},
    output::{Output, StatusCode},
    system::System,
    uints::{U256, U512},
};

#[macro_export]
macro_rules! abort {
  ($code:ident, $msg:literal $(, $ex:expr)*) => {
      fvm_sdk::vm::abort(
          fvm_shared::error::ExitCode::$code.value(),
          Some(format!($msg, $($ex,)*).as_str()),
      )
  };
}
