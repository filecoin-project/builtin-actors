#![allow(dead_code)]

use {
    crate::interpreter::transaction::TransactionAction,
    crate::interpreter::SignedTransaction,
    crate::interpreter::{H160, U256},
    bytes::Bytes,
    fil_actors_runtime::ActorError,
    std::fmt::Debug,
};

/// The kind of call-like instruction.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CallKind {
    Call,
    DelegateCall,
    CallCode,
    Create,
    Create2 { salt: U256 },
}

/// The message describing an EVM call,
/// including a zero-depth call from transaction origin.
#[derive(Clone, Debug, PartialEq)]
pub struct Message {
    /// The kind of the call. For zero-depth calls `CallKind::Call` SHOULD be
    /// used.
    pub kind: CallKind,

    /// Static call mode.
    pub is_static: bool,

    /// The call depth.
    pub depth: i32,

    /// The amount of gas for message execution.
    pub gas: i64,

    /// The destination (recipient) of the message.
    pub recipient: H160,

    /// The sender of the message.
    pub sender: H160,

    /// Message input data.
    pub input_data: Bytes,

    /// The amount of Ether transferred with the message.
    pub value: U256,
}

impl TryFrom<SignedTransaction> for Message {
    type Error = ActorError;

    fn try_from(tx: SignedTransaction) -> Result<Self, Self::Error> {
        Ok(Message {
            kind: match tx.transaction.action() {
                TransactionAction::Call(_) => CallKind::Call,
                TransactionAction::Create => CallKind::Create,
            },
            is_static: false,
            depth: 0,
            gas: tx.transaction.gas_limit() as i64,
            recipient: match tx.transaction.action() {
                TransactionAction::Call(addr) => addr,
                TransactionAction::Create => H160::zero(),
            },
            sender: tx.sender_address()?,
            input_data: tx.transaction.input(),
            value: tx.transaction.value(),
        })
    }
}

/// This type is used to construct a new instance of an EVM contract.
/// Instances of this type are created by the bridge actor after a successful
/// invocation of EVM contract constructor.
#[derive(serde_tuple::Serialize_tuple, serde_tuple::Deserialize_tuple)]
pub struct EvmContractRuntimeConstructor {
    pub initial_state: cid::Cid,
    pub bytecode: bytes::Bytes,
    pub registry: fvm_shared::address::Address,
    pub address: H160,
}

impl Debug for EvmContractRuntimeConstructor {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EvmContractRuntimeConstructor")
            .field("initial_state", &self.initial_state.to_string())
            .field("bytecode", &hex::encode(&self.bytecode))
            .field("registry", &self.registry)
            .field("address", &self.address)
            .finish()
    }
}
