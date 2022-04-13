use fvm_ipld_encoding::tuple::*;
use fvm_shared::clock::ChainEpoch;

pub const CROSSMSG_AMT_BITWIDTH: u32 = 3;
pub const DEFAULT_CHECKPOINT_PERIOD: ChainEpoch = 10;
pub const MAX_NONCE: u64 = 1 << 63;
pub const MIN_COLLATERAL_AMOUNT: u64 = 10_u64.pow(18);

#[derive(Serialize_tuple, Deserialize_tuple)]
pub struct ConstructorParams {
    pub network_name: String,
    pub checkpoint_period: ChainEpoch,
}

#[derive(Serialize_tuple, Deserialize_tuple)]
pub struct SubnetIDParam {
    pub id: String,
}
