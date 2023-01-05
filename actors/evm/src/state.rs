use fvm_shared::ActorID;

use {
    cid::Cid,
    fvm_ipld_encoding::tuple::*,
    fvm_ipld_encoding::Cbor,
    serde_tuple::{Deserialize_tuple, Serialize_tuple},
};

/// A tombstone indicating that the contract has been self-destructed.
#[derive(Copy, Clone, Debug, Eq, PartialEq, Serialize_tuple, Deserialize_tuple)]
pub struct Tombstone {
    /// The message origin when this actor was self-destructed.
    pub origin: ActorID,
    /// The message nonce when this actor was self-destructed.
    pub nonce: u64,
}

/// Data stored by an EVM contract.
/// This runs on the fvm-evm-runtime actor code cid.
#[derive(Debug, Serialize_tuple, Deserialize_tuple)]
pub struct State {
    /// The EVM contract bytecode resulting from calling the
    /// initialization code by the constructor.
    pub bytecode: Cid,

    /// The EVM contract bytecode hash keccak256(bytecode)
    pub bytecode_hash: multihash::Multihash,

    /// The EVM contract state dictionary.
    /// All eth contract state is a map of U256 -> U256 values.
    ///
    /// KAMT<U256, U256>
    pub contract_state: Cid,

    /// The EVM nonce used to track how many times CREATE or CREATE2 have been called.
    pub nonce: u64,

    /// Possibly a tombstone if this actor has been self-destructed.
    pub tombstone: Option<Tombstone>,
}

impl Cbor for State {}
