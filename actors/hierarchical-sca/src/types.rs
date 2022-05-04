use fil_actors_runtime::Array;
use fvm_ipld_encoding::tuple::*;
use fvm_shared::bigint::bigint_ser;
use fvm_shared::clock::ChainEpoch;
use fvm_shared::econ::TokenAmount;

use crate::checkpoint::{Checkpoint, CrossMsgMeta};
use crate::StorableMsg;

pub const CROSSMSG_AMT_BITWIDTH: u32 = 3;
pub const DEFAULT_CHECKPOINT_PERIOD: ChainEpoch = 10;
pub const MAX_NONCE: u64 = 1 << 63;
pub const MIN_COLLATERAL_AMOUNT: u64 = 10_u64.pow(18);

pub type CrossMsgMetaArray<'bs, BS> = Array<'bs, CrossMsgMeta, BS>;
pub type CrossMsgArray<'bs, BS> = Array<'bs, StorableMsg, BS>;

#[derive(Serialize_tuple, Deserialize_tuple)]
pub struct ConstructorParams {
    pub network_name: String,
    pub checkpoint_period: ChainEpoch,
}

#[derive(Serialize_tuple, Deserialize_tuple, Clone)]
pub struct FundParams {
    #[serde(with = "bigint_ser")]
    pub value: TokenAmount,
}

#[derive(Debug, Serialize_tuple, Deserialize_tuple)]
pub struct CheckpointParams {
    pub checkpoint: Checkpoint,
}
