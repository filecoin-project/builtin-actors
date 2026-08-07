// Copyright 2019-2022 ChainSafe Systems
// SPDX-License-Identifier: Apache-2.0, MIT

use fvm_ipld_encoding::tuple::*;
use fvm_shared::address::Address;
use fvm_shared::bigint::bigint_ser::BigIntDe;
use fvm_shared::clock::ChainEpoch;
use fvm_shared::econ::TokenAmount;

use crate::{DistributionInit, PendingWriteOp, RecipientShare, StreamId, WeightRecordUpdate};

#[derive(Clone, Debug, PartialEq, Eq, Serialize_tuple, Deserialize_tuple)]
#[serde(transparent)]
pub struct ConstructorParams {
    pub power: Option<BigIntDe>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize_tuple, Deserialize_tuple)]
pub struct AwardBlockRewardParams {
    pub miner: Address,
    pub penalty: TokenAmount,
    pub gas_reward: TokenAmount,
    pub win_count: i64,
}

pub use fil_actors_runtime::builtin::reward::ThisEpochRewardReturn;

#[derive(Clone, Debug, PartialEq, Eq, Serialize_tuple, Deserialize_tuple)]
#[serde(transparent)]
pub struct UpdateNetworkKPIParams {
    pub curr_realized_power: Option<BigIntDe>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize_tuple, Deserialize_tuple)]
pub struct SetWeightRecordsParams {
    pub updates: Vec<WeightRecordUpdate>,
}

pub type StepWeightRecordsParams = SetWeightRecordsParams;

#[derive(Clone, Debug, PartialEq, Eq, Serialize_tuple, Deserialize_tuple)]
pub struct RegisterStreamParams {
    pub id: StreamId,
    pub weight: crate::WeightRecord,
    pub distribution: Option<DistributionInit>,
    pub activation_epoch: ChainEpoch,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize_tuple, Deserialize_tuple)]
pub struct RemoveStreamParams {
    pub id: StreamId,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize_tuple, Deserialize_tuple)]
pub struct SetDistributionParams {
    pub id: StreamId,
    pub writer: Address,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize_tuple, Deserialize_tuple)]
pub struct SetSharesParams {
    pub id: StreamId,
    pub shares: Vec<RecipientShare>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize_tuple, Deserialize_tuple)]
pub struct CancelPendingParams {
    pub id: Option<StreamId>,
    pub op: PendingWriteOp,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize_tuple, Deserialize_tuple)]
pub struct ClaimParams {
    pub id: StreamId,
    pub wallets: Vec<Address>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize_tuple, Deserialize_tuple)]
pub struct ClaimReturn {
    pub amounts: Vec<TokenAmount>,
}
