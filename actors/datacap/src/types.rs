use fvm_ipld_encoding::tuple::*;
use fvm_ipld_encoding::{Cbor, RawBytes};
use fvm_shared::address::Address;
use fvm_shared::bigint::{bigint_ser, BigInt};

#[derive(Clone, Debug, PartialEq, Eq, Serialize_tuple, Deserialize_tuple)]
pub struct AllowanceParams {
    pub owner: Address,
    pub operator: Address,
}

impl Cbor for AllowanceParams {}

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

#[derive(Clone, Debug, PartialEq, Eq, Serialize_tuple, Deserialize_tuple)]
pub struct TransferParams {
    pub to: Address,
    #[serde(with = "bigint_ser")]
    pub amount: BigInt,
    pub data: RawBytes,
}

impl Cbor for TransferParams {}

#[derive(Clone, Debug, PartialEq, Eq, Serialize_tuple, Deserialize_tuple)]
pub struct TransferFromParams {
    pub from: Address,
    pub to: Address,
    #[serde(with = "bigint_ser")]
    pub amount: BigInt,
    pub data: RawBytes,
}

impl Cbor for TransferFromParams {}

#[derive(Clone, Debug, PartialEq, Eq, Serialize_tuple, Deserialize_tuple)]
pub struct IncreaseAllowanceParams {
    pub operator: Address,
    #[serde(with = "bigint_ser")]
    pub amount: BigInt,
}

impl Cbor for IncreaseAllowanceParams {}

#[derive(Clone, Debug, PartialEq, Eq, Serialize_tuple, Deserialize_tuple)]
pub struct DecreaseAllowanceParams {
    pub operator: Address,
    #[serde(with = "bigint_ser")]
    pub amount: BigInt,
}

impl Cbor for DecreaseAllowanceParams {}

#[derive(Clone, Debug, PartialEq, Eq, Serialize_tuple, Deserialize_tuple)]
pub struct RevokeAllowanceParams {
    pub operator: Address,
}

impl Cbor for RevokeAllowanceParams {}

#[derive(Clone, Debug, PartialEq, Eq, Serialize_tuple, Deserialize_tuple)]
pub struct BurnParams {
    #[serde(with = "bigint_ser")]
    pub amount: BigInt,
}

impl Cbor for BurnParams {}

#[derive(Clone, Debug, PartialEq, Eq, Serialize_tuple, Deserialize_tuple)]
pub struct BurnFromParams {
    pub from: Address,
    #[serde(with = "bigint_ser")]
    pub amount: BigInt,
}

impl Cbor for BurnFromParams {}
