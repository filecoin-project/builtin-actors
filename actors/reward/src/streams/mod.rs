// Copyright 2019-2022 ChainSafe Systems
// SPDX-License-Identifier: Apache-2.0, MIT

use fvm_ipld_encoding::tuple::*;
use fvm_shared::econ::TokenAmount;

mod award;
mod distribution;
mod invariants;
mod queue;
mod weights;

pub use self::award::{RewardAllocation, compute_service_liability};
pub(crate) use self::award::{accrue_service, allocate_reward};
pub use self::distribution::{
    DistributionInit, ExplicitDistribution, RecipientAmount, RecipientShare,
};
pub(crate) use self::distribution::{claim, normalize_shares, set_shares};
pub(crate) use self::invariants::{
    validate_award_state_structure, validate_mutation_state, validate_streams_state,
};
pub use self::queue::{
    ApplyResult, PendingWrite, PendingWriteOp, RegisterStreamPayload, SetDistributionPayload,
};
pub(crate) use self::queue::{
    apply_due_writes, apply_due_writes_and_cancel, queue_register_stream, queue_remove_stream,
    queue_set_distribution, queue_weight_records, validate_cancel_target,
};
pub use self::weights::{WeightRecord, WeightRecordUpdate};

pub type StreamId = u64;

pub const DENOM: u64 = 1_000_000_000_000_000_000;
pub const MAX_STREAMS: usize = 8;
pub const MAX_RECIPIENTS: usize = 64;
pub const MAX_PAYABLE_ROWS_PER_STREAM: usize = 2 * MAX_RECIPIENTS;
pub const MAX_TOMBSTONE_ROWS: usize = 256;
const STREAM_SCOPED_PENDING_OPS: usize = 3;
const SCHEDULE_WIDE_PENDING_OPS: usize = 2;
pub const MAX_PENDING_WRITES: usize =
    MAX_STREAMS * STREAM_SCOPED_PENDING_OPS + SCHEDULE_WIDE_PENDING_OPS;

/// A live stream persisted in `StreamsState`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize_tuple, Deserialize_tuple)]
pub struct Stream {
    pub id: StreamId,
    pub weight: WeightRecord,
    /// None is the implicit consensus distribution; Some is an explicit service distribution.
    pub distribution: Option<ExplicitDistribution>,
}

/// Persisted liabilities for a removed stream.
#[derive(Clone, Debug, PartialEq, Eq, Serialize_tuple, Deserialize_tuple)]
pub struct Tombstone {
    pub id: StreamId,
    pub payable: Vec<RecipientAmount>,
}

/// Stream state persisted as the block referenced by `State.streams_root`.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize_tuple, Deserialize_tuple)]
pub struct StreamsState {
    /// Ordered by stream ID.
    pub streams: Vec<Stream>,
    /// Ordered by stream ID.
    pub tombstones: Vec<Tombstone>,
    /// Ordered by effective epoch; equal epochs retain queue position.
    pub pending_writes: Vec<PendingWrite>,
}

/// Current-period gross accrual persisted inline in `State`.
///
/// It stays outside `StreamsState` because it changes on every award.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize_tuple, Deserialize_tuple)]
pub struct StreamAccrual {
    pub id: StreamId,
    pub amount: TokenAmount,
}

#[cfg(test)]
mod tests;
