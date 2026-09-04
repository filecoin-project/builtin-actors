// Copyright 2019-2022 ChainSafe Systems
// SPDX-License-Identifier: Apache-2.0, MIT

//! Method parameter and return tuples, and the payload tuples a queued SWA call carries
//! between admission and application (FIP-0118 2.4.9).

use fvm_ipld_encoding::tuple::*;
use fvm_shared::address::Address;
use fvm_shared::bigint::bigint_ser::BigIntDe;
use fvm_shared::clock::ChainEpoch;
use fvm_shared::econ::TokenAmount;

use crate::state::{PendingWriteOp, RecipientShare, StreamId, WeightRecord};

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
    pub weight: WeightRecord,
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

/// One stream update in `SetWeightRecords` and `StepWeightRecords` parameters.
#[derive(Clone, Debug, PartialEq, Eq, Serialize_tuple, Deserialize_tuple)]
pub struct WeightRecordUpdate {
    pub id: StreamId,
    pub weight: WeightRecord,
}

/// The tuple stored in `PendingWrite.payload` for `SetWeightRecords` and `StepWeightRecords`.
///
/// This is the deferred call's own payload, not the weight-management parameter tuples, which
/// carry the same updates in their own shape.
#[derive(Clone, Debug, PartialEq, Eq, Serialize_tuple, Deserialize_tuple)]
pub struct WeightRecordsPayload {
    pub updates: Vec<WeightRecordUpdate>,
}

/// Caller-supplied subset of a new explicit distribution.
#[derive(Clone, Debug, PartialEq, Eq, Serialize_tuple, Deserialize_tuple)]
pub struct DistributionInit {
    pub writer: Address,
    pub shares: Vec<RecipientShare>,
}

/// The tuple stored in `PendingWrite.payload` for `RegisterStream`.
///
/// This is the deferred call's own payload, not the `RegisterStream` method parameter tuple.
#[derive(Clone, Debug, PartialEq, Eq, Serialize_tuple, Deserialize_tuple)]
pub struct RegisterStreamPayload {
    pub weight: WeightRecord,
    pub distribution: Option<DistributionInit>,
}

/// The tuple stored in `PendingWrite.payload` for `SetDistribution`.
///
/// This is the deferred call's own payload, not the `SetDistribution` method parameter tuple.
#[derive(Clone, Debug, PartialEq, Eq, Serialize_tuple, Deserialize_tuple)]
pub struct SetDistributionPayload {
    pub writer: Address,
}
