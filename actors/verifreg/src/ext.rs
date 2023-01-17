use fvm_ipld_encoding::serde_bytes;
use fvm_ipld_encoding::tuple::*;
use fvm_ipld_encoding::tuple::{Deserialize_tuple, Serialize_tuple};
use fvm_shared::address::Address;

pub mod account {
    use super::*;

    pub const AUTHENTICATE_MESSAGE_METHOD: u64 =
        frc42_dispatch::method_hash!("AuthenticateMessage");

    #[derive(Serialize_tuple, Deserialize_tuple)]
    pub struct AuthenticateMessageParams {
        #[serde(with = "serde_bytes")]
        pub signature: Vec<u8>,
        #[serde(with = "serde_bytes")]
        pub message: Vec<u8>,
    }
}

pub mod datacap {
    use super::*;
    use fvm_shared::econ::TokenAmount;

    #[repr(u64)]
    pub enum Method {
        Mint = frc42_dispatch::method_hash!("Mint"),
        Destroy = frc42_dispatch::method_hash!("Destroy"),
        Balance = frc42_dispatch::method_hash!("Balance"),
        Transfer = frc42_dispatch::method_hash!("Transfer"),
        Burn = frc42_dispatch::method_hash!("Burn"),
    }

    #[derive(Clone, Debug, PartialEq, Eq, Serialize_tuple, Deserialize_tuple)]
    pub struct MintParams {
        pub to: Address,
        pub amount: TokenAmount,
        pub operators: Vec<Address>,
    }

    #[derive(Clone, Debug, PartialEq, Eq, Serialize_tuple, Deserialize_tuple)]
    pub struct DestroyParams {
        pub owner: Address,
        pub amount: TokenAmount,
    }
}
