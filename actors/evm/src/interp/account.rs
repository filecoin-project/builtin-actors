use {
    crate::interp::uints::U256,
    fvm_shared::address::Address as FileCoinAddress,
    serde::{Deserialize, Serialize},
    serde_tuple::{Deserialize_tuple, Serialize_tuple},
};

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum AccountKind {
    /// A user account controlled by a private key.
    ///
    /// Has no contract code or state. May optionally
    /// be linked to FIL account.
    ExternallyOwned { fil_account: Option<FileCoinAddress> },

    /// A contract account.
    ///
    /// Has state and bytecode but no corresponding private key.
    /// Always known for contract accounts, because all contract
    /// creation goes through the registry.
    Contract { fil_account: FileCoinAddress },
}

impl Default for AccountKind {
    fn default() -> Self {
        Self::ExternallyOwned { fil_account: None }
    }
}

/// Represents an account in the EVM world addressable by a 160 bit address.
///
/// An account can be either an externally owned account (EOA) that is
/// controlled by a private key or a contract address. The fact that ethereum
/// addresses are hashes of the public key, makes it impossible to distinguish
/// between EOA and contract accounts.
///
/// If an account is an EOA it is optionally linked to a FileCoin account,
/// in that case, the balance of that account is the sum of balances on the
/// FVM side (if known) and EVM side, and the nonce value is the higher of
/// the two. The mapping between FVM and EVM addresses is a manual step and
/// not all ETH addresses will have their FVM secp256k1 equivalent.
///
/// If an account is a contract account, then it may be linked to an FVM
/// account that is an Actor object constructed with the contract bytecode
/// and has its CodeCID equal to the runtime wasm bytecode.
#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize_tuple, Deserialize_tuple)]
pub struct EthereumAccount {
    /// For EOA it could be this value of the FIL account nonce,
    /// whichever is greater. If this account is not linked to a FIL
    /// account, then this nonce value alone is used as the nonce.
    pub nonce: u64,

    /// The FIL balance of this account.
    ///
    /// For EAO if the corresponding FIL account is known then the balance is the
    /// sum of the two balances, otherwise this value alone is the balance.
    pub balance: U256,

    /// Type of the Ethereum account.
    ///
    /// Contract accounts have a state object (HAMT) and a state root,
    /// EOA may optionally have a link to a FIL address.
    pub kind: AccountKind,
}
