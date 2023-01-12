use std::array::TryFromSliceError;

use fvm_shared::ActorID;

use {
    cid::Cid,
    fvm_ipld_encoding::strict_bytes,
    fvm_ipld_encoding::tuple::*,
    serde::{Deserialize, Serialize},
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

/// A Keccak256 digest of EVM bytecode.
#[derive(Deserialize, Serialize, Clone, Copy, Eq, PartialEq)]
#[serde(transparent)]
pub struct BytecodeHash(#[serde(with = "strict_bytes")] [u8; 32]);

impl std::fmt::Debug for BytecodeHash {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_tuple("BytecodeHash").field(&format_args!("{}", self)).finish()
    }
}

impl std::fmt::Display for BytecodeHash {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if f.alternate() {
            write!(f, "0x")?;
        }
        for b in self.0 {
            write!(f, "{b:02X}")?;
        }
        Ok(())
    }
}

impl BytecodeHash {
    pub const ZERO: Self = Self([0; 32]);

    /// Keccak256 hash of `[0xfe]`, "native bytecode"
    pub const NATIVE_ACTOR: Self =
        Self(hex_literal::hex!("bcc90f2d6dada5b18e155c17a1c0a55920aae94f39857d39d0d8ed07ae8f228b"));

    /// Keccak256 hash of `[]`, empty bytecode
    pub const EMPTY: Self =
        Self(hex_literal::hex!("c5d2460186f7233c927e7db2dcc703c0e500b653ca82273b7bfad8045d85a470"));

    pub fn as_slice(&self) -> &[u8] {
        &self.0
    }
}

impl From<[u8; 32]> for BytecodeHash {
    fn from(digest: [u8; 32]) -> Self {
        BytecodeHash(digest)
    }
}

impl From<BytecodeHash> for [u8; 32] {
    fn from(digest: BytecodeHash) -> Self {
        digest.0
    }
}

impl From<BytecodeHash> for Vec<u8> {
    fn from(digest: BytecodeHash) -> Self {
        digest.0.into()
    }
}

impl TryFrom<&[u8]> for BytecodeHash {
    type Error = TryFromSliceError;

    fn try_from(value: &[u8]) -> Result<Self, TryFromSliceError> {
        Ok(Self(value.try_into()?))
    }
}

/// Data stored by an EVM contract.
/// This runs on the fvm-evm-runtime actor code cid.
#[derive(Debug, Serialize_tuple, Deserialize_tuple)]
pub struct State {
    /// The EVM contract bytecode resulting from calling the
    /// initialization code by the constructor.
    pub bytecode: Cid,

    /// The EVM contract bytecode hash keccak256(bytecode)
    pub bytecode_hash: BytecodeHash,

    /// The EVM contract state dictionary.
    /// All eth contract state is a map of U256 -> U256 values.
    ///
    /// KAMT<U256, U256>
    pub contract_state: Cid,

    /// The EVM nonce used to track how many times CREATE or CREATE2 have been called.
    pub nonce: u64,

    /// Possibly a tombstone if this actor has been self-destructed.
    ///
    /// In the EVM, self-destructed contracts are "alive" until the current top-level transaction
    /// ends. We track this by recording the origin and nonce.
    ///
    /// Specifically:
    ///
    /// 1. On SELFDESTRUCT, they mark themselves as "deleted" (by setting a tombstone with the
    ///    current origin/nonce), send away all funds, and return immediately.
    /// 2. For the rest of the current transaction (as long as the tombstone's origin/nonce matches
    ///    the currently executing top-level transaction) , the contract continues to behave
    ///    normally.
    /// 3. After the current transaction ends, the contract behaves as if it were an "empty"
    ///    contract, kind of like an embryo. At this point, the contract can be "resurrected"
    ///    (recreated) by via CREATE/CREATE2.
    ///
    /// See https://github.com/filecoin-project/ref-fvm/issues/1174 for some context.
    pub tombstone: Option<Tombstone>,
}

#[cfg(test)]
mod test {
    use fvm_ipld_encoding::{from_slice, to_vec, BytesDe};

    use crate::BytecodeHash;
    #[test]
    fn test_bytecode_hash_serde() {
        let encoded = to_vec(&BytecodeHash::EMPTY).unwrap();
        let BytesDe(decoded) = from_slice(&encoded).unwrap();
        assert_eq!(BytecodeHash::try_from(&decoded[..]).unwrap(), BytecodeHash::EMPTY);
    }

    #[test]
    fn test_bytecode_hash_format() {
        assert_eq!(
            BytecodeHash::ZERO.to_string(),
            "0000000000000000000000000000000000000000000000000000000000000000"
        );
        assert_eq!(
            format!("{:#}", BytecodeHash::ZERO),
            "0x0000000000000000000000000000000000000000000000000000000000000000"
        );

        assert_eq!(
            format!("{:?}", BytecodeHash::ZERO),
            "BytecodeHash(0000000000000000000000000000000000000000000000000000000000000000)"
        );
    }
}
