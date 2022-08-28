use fvm_ipld_encoding::tuple::*;
use fvm_ipld_encoding::Cbor;
use fvm_shared::address::Address;
use fvm_shared::bigint::{bigint_ser, BigInt};

#[derive(Clone, Debug, PartialEq, Eq, Serialize_tuple, Deserialize_tuple)]
pub struct MintParams {
    pub to: Address,
    #[serde(with = "bigint_ser")]
    pub amount: BigInt,
}

impl Cbor for MintParams {}

#[derive(Clone, Debug, PartialEq, Eq, Serialize_tuple, Deserialize_tuple)]
pub struct DestroyParams {
    pub owner: Address,
    #[serde(with = "bigint_ser")]
    pub amount: BigInt,
}

impl Cbor for DestroyParams {}
