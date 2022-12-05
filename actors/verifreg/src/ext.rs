use fvm_ipld_encoding::tuple::*;
use fvm_ipld_encoding::tuple::{Deserialize_tuple, Serialize_tuple};
use fvm_shared::address::Address;

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
