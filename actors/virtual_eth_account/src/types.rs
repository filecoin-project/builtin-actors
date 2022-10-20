use fvm_ipld_encoding::tuple::*;
use fvm_ipld_encoding::{serde_bytes, Cbor};
use fvm_shared::address::Address;
use fvm_shared::econ::TokenAmount;
use fvm_shared::MethodNum;

#[derive(Debug, Serialize_tuple, Deserialize_tuple)]
pub struct AuthenticateMessageParams {
    #[serde(with = "serde_bytes")]
    pub signature: Vec<u8>,
    #[serde(with = "serde_bytes")]
    pub message: Vec<u8>,
}

impl Cbor for AuthenticateMessageParams {}

#[derive(Debug, Serialize_tuple, Deserialize_tuple)]
pub struct ForwardParams {
    pub to: Address,
    pub method: MethodNum,
    #[serde(with = "serde_bytes")]
    pub params: Vec<u8>,
    pub value: TokenAmount,
}

impl Cbor for ForwardParams {}

#[derive(Debug, Serialize_tuple, Deserialize_tuple)]
pub struct ForwardValueParams {
    pub to: Address,
    pub value: TokenAmount,
}

impl Cbor for ForwardValueParams {}
