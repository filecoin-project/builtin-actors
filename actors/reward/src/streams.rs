// Copyright 2019-2022 ChainSafe Systems
// SPDX-License-Identifier: Apache-2.0, MIT

use fvm_ipld_encoding::RawBytes;
use fvm_ipld_encoding::repr::*;
use fvm_ipld_encoding::tuple::*;
use fvm_shared::address::Address;
use fvm_shared::clock::ChainEpoch;
use fvm_shared::econ::TokenAmount;

pub type StreamId = u64;

/// Identifies the SWA operation captured by a deferred write.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize_repr, Deserialize_repr)]
#[repr(u8)]
pub enum PendingWriteOp {
    SetWeightRecords = 0,
    /// Gate-originated weight update; unlike other operations, it cannot be cancelled.
    StepWeightRecords = 1,
    RegisterStream = 2,
    RemoveStream = 3,
    SetDistribution = 4,
}

/// A clamped linear weight in DENOM fixed point. The per-epoch slope is signed.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize_tuple, Deserialize_tuple)]
pub struct WeightRecord {
    pub v_start: u64,
    pub slope: i64,
    pub t_start: ChainEpoch,
    pub floor: u64,
    pub cap: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize_tuple, Deserialize_tuple)]
pub struct RecipientShare {
    pub recipient: Address,
    pub share: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize_tuple, Deserialize_tuple)]
pub struct ExplicitDistribution {
    /// Designated share writer, not a payee.
    pub writer: Address,
    /// Current recipient fractions for the open share period.
    pub shares: Vec<RecipientShare>,
    /// Unclaimed allocations carried from closed share periods.
    pub payable: Vec<RecipientAmount>,
    /// Amounts already claimed against the current period's gross accrual.
    pub claimed_period: Vec<RecipientAmount>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize_tuple, Deserialize_tuple)]
pub struct RecipientAmount {
    pub recipient: Address,
    pub amount: TokenAmount,
}

/// A currently registered stream.
#[derive(Clone, Debug, PartialEq, Eq, Serialize_tuple, Deserialize_tuple)]
pub struct Stream {
    pub id: StreamId,
    pub weight: WeightRecord,
    /// None is the implicit consensus distribution; Some is an explicit service distribution.
    pub distribution: Option<ExplicitDistribution>,
}

/// Outstanding liabilities preserved after a stream is removed.
#[derive(Clone, Debug, PartialEq, Eq, Serialize_tuple, Deserialize_tuple)]
pub struct Tombstone {
    pub id: StreamId,
    pub payable: Vec<RecipientAmount>,
}

/// An operation captured for execution after the SWA timelock.
#[derive(Clone, Debug, PartialEq, Eq, Serialize_tuple, Deserialize_tuple)]
pub struct PendingWrite {
    pub id: StreamId,
    pub op: PendingWriteOp,
    /// Operation-specific CBOR tuple.
    pub payload: RawBytes,
    pub effective_epoch: ChainEpoch,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize_tuple, Deserialize_tuple)]
pub struct StreamsState {
    /// Ordered by stream ID.
    pub streams: Vec<Stream>,
    /// Ordered by stream ID.
    pub tombstones: Vec<Tombstone>,
    /// Ordered by effective epoch, stream ID, then operation.
    pub pending_writes: Vec<PendingWrite>,
}

/// Current-period gross accrual for a live explicit stream.
///
/// Stored inline in State, rather than StreamsState, because it changes on every award.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize_tuple, Deserialize_tuple)]
pub struct StreamAccrual {
    pub id: StreamId,
    pub amount: TokenAmount,
}
