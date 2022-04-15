use cid::Cid;
use fvm_ipld_encoding::tuple::*;
use fvm_ipld_encoding::{serde_bytes, Cbor};
use fvm_shared::bigint::bigint_ser;
use fvm_shared::clock::ChainEpoch;
use fvm_shared::econ::TokenAmount;

#[derive(Default, PartialEq, Eq, Clone, Debug, Serialize_tuple, Deserialize_tuple)]
pub struct Checkpoint {
    data: CheckData,
    #[serde(with = "serde_bytes")]
    sig: Vec<u8>,
}
impl Cbor for Checkpoint {}

#[derive(Default, PartialEq, Eq, Clone, Debug, Serialize_tuple, Deserialize_tuple)]
pub struct CheckData {
    source: String,
    #[serde(with = "serde_bytes")]
    tip_set: Vec<u8>,
    epoch: ChainEpoch,
    prev_check: Cid,
    childs: Vec<ChildCheck>,
    cross_msgs: Vec<CrossMsgs>,
}

impl Cbor for CheckData {}

#[derive(PartialEq, Eq, Clone, Debug, Serialize_tuple, Deserialize_tuple)]
pub struct CrossMsgs {
    from: String,
    to: String,
    msgs_cid: Cid,
    nonce: u64,
    #[serde(with = "bigint_ser")]
    value: TokenAmount,
}
impl Cbor for CrossMsgs {}

#[derive(PartialEq, Eq, Clone, Debug, Serialize_tuple, Deserialize_tuple)]
pub struct ChildCheck {
    source: String,
    checks: Vec<Cid>,
}
impl Cbor for ChildCheck {}
