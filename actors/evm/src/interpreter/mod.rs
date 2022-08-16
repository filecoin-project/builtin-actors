pub mod account;
pub mod bytecode;
pub mod execution;
pub mod instructions;
pub mod memory;
pub mod message;
pub mod opcode;
pub mod output;
mod precompiles;
pub mod stack;
pub mod system;
pub mod transaction;
pub mod uints;

pub use {
    account::{AccountKind, EthereumAccount},
    bytecode::Bytecode,
    execution::{execute, ExecutionState},
    message::{CallKind, Message},
    output::{Output, StatusCode},
    system::System,
    transaction::{
        SignedTransaction, Transaction, TransactionAction, TransactionRecoveryId,
        TransactionSignature,
    },
    uints::{H160, H256, U256, U512},
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
