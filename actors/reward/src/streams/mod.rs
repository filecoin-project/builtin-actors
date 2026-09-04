//! Stream state: the block behind `State.streams_root`, plus the accrual rows in the state root.
//!
//! FIP-0118 section 2.4.2 defines the layout as implemented here.
//!
//! 2.4.2 also defines ordering: "`accrued`, `streams`, and `tombstones` have unique ascending
//! stream IDs; recipient tables have unique ascending recipient IDs; and `pending_writes` is
//! ordered by effective epoch, preserving admission order at equal epochs."
//!
//! A stream's distribution is one of two kinds (2.4.1). `IMPLICIT`, the `None` arm, stores
//! nothing and pays the block winner; only the consensus stream is implicit. `EXPLICIT`, the
//! `Some` arm, carries a writer and three wallet-keyed tables. The FIP-0118 migration pins
//! consensus = 1 and the service stream = 2, but f02 only knows and cares about the kind.

use anyhow::Result;
use fil_actors_runtime::runtime::Runtime;
use fil_actors_runtime::{ActorDowncast, ActorError, actor_error};
use fvm_ipld_encoding::tuple::*;
use fvm_ipld_encoding::{CborStore, from_slice};
use fvm_shared::clock::ChainEpoch;
use fvm_shared::econ::TokenAmount;
use fvm_shared::error::ExitCode;
use multihash_codetable::Code;

use crate::State;

mod award;
mod distribution;
mod invariants;
mod queue;
mod weights;

pub use self::award::{RewardAllocation, explicit_liability};
pub(crate) use self::award::{accrue_explicit, allocate_reward};
pub use self::distribution::{
    DistributionInit, ExplicitDistribution, RecipientAmount, RecipientShare, RecipientTable,
};
pub(crate) use self::distribution::{claim, normalize_shares, set_shares};
pub(crate) use self::invariants::validate_streams_state;
pub use self::queue::{
    ApplyResult, PendingWrite, PendingWriteOp, RegisterStreamPayload, SetDistributionPayload,
};
pub(crate) use self::queue::{
    CancelResult, Slot, queue_register_stream, queue_remove_stream, queue_set_distribution,
    queue_weight_records,
};
pub use self::weights::{WeightRecord, WeightRecordUpdate, WeightRecordsPayload};

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
    /// None is the implicit consensus distribution; Some is an explicit distribution.
    pub distribution: Option<ExplicitDistribution>,
}

impl Stream {
    /// True for the consensus stream, whose portion pays the block winner directly.
    pub(crate) fn is_implicit(&self) -> bool {
        self.distribution.is_none()
    }

    /// The stored writer and recipient tables, for an explicit stream.
    pub(crate) fn explicit(&self) -> Option<&ExplicitDistribution> {
        self.distribution.as_ref()
    }

    pub(crate) fn explicit_mut(&mut self) -> Option<&mut ExplicitDistribution> {
        self.distribution.as_mut()
    }
}

/// Persisted liabilities for a removed stream.
#[derive(Clone, Debug, PartialEq, Eq, Serialize_tuple, Deserialize_tuple)]
pub struct Tombstone {
    pub id: StreamId,
    pub payable: RecipientTable,
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

/// Stream state that has passed the structure and accounting invariants.
///
/// Construction is the only place either invariant runs, and it is the only way to obtain one, so
/// an operation cannot be handed state it needs to re-check. The schedule invariants are
/// deliberately not checked: claims, cancellation and `SetShares` stay usable while the weight
/// schedule is invalid (FIP-0118 2.4.8); the schedule invariants are checked separately.
///
/// The accrual rows are part of the ledger even though they persist in the state root rather than
/// the streams block, because every liability the block describes is measured against them.
pub(crate) struct Ledger {
    streams: StreamsState,
    accrued: Vec<StreamAccrual>,
    /// Set by the paths that can change the streams block, which [`Ledger::store`] then writes.
    streams_dirty: bool,
}

impl Ledger {
    /// Loads and validates the block behind `streams_root`; anything invalid is illegal state.
    pub(crate) fn load(rt: &impl Runtime, st: &State) -> Result<Ledger, ActorError> {
        let streams: StreamsState = rt
            .store()
            .get_cbor(&st.streams_root)
            .map_err(|e| {
                e.downcast_default(ExitCode::USR_ILLEGAL_STATE, "failed to load streams state")
            })?
            .ok_or_else(|| {
                actor_error!(illegal_state, "streams state root {} not found", st.streams_root)
            })?;
        Ledger::checked(streams, st.accrued.clone()).map_err(|e| {
            e.downcast_default(ExitCode::USR_ILLEGAL_STATE, "invalid persisted stream state")
        })
    }

    /// Decodes and validates the streams block for the award, whose every failure pays gas only.
    pub(crate) fn decode_for_award(bytes: &[u8], accrued: &[StreamAccrual]) -> Result<Ledger> {
        Ledger::checked(from_slice(bytes)?, accrued.to_vec())
    }

    fn checked(streams: StreamsState, accrued: Vec<StreamAccrual>) -> Result<Ledger> {
        invariants::structure(&streams)?;
        invariants::accounting(&streams, &accrued)?;
        Ok(Ledger { streams, accrued, streams_dirty: false })
    }

    /// Writes the accrual rows, and the streams block when this ledger has touched it.
    pub(crate) fn store(&self, rt: &impl Runtime, st: &mut State) -> Result<(), ActorError> {
        st.accrued = self.accrued.clone();
        if self.streams_dirty {
            st.streams_root =
                rt.store().put_cbor(&self.streams, Code::Blake2b256).map_err(|e| {
                    e.downcast_default(ExitCode::USR_ILLEGAL_STATE, "failed to store streams state")
                })?;
        }
        Ok(())
    }

    pub(crate) fn streams(&self) -> &StreamsState {
        &self.streams
    }

    pub(crate) fn accrued(&self) -> &[StreamAccrual] {
        &self.accrued
    }

    /// The accrual rows alone, which the award credits without touching the streams block.
    pub(crate) fn accrued_mut(&mut self) -> &mut Vec<StreamAccrual> {
        &mut self.accrued
    }

    /// Both halves, for an operation that moves value between them and rewrites the block.
    pub(crate) fn mutate(&mut self) -> (&mut StreamsState, &mut Vec<StreamAccrual>) {
        self.streams_dirty = true;
        (&mut self.streams, &mut self.accrued)
    }

    /// Applies every write due through `epoch`, which every method does before its own work.
    pub(crate) fn apply_due(&mut self, epoch: ChainEpoch) -> Result<ApplyResult> {
        let result = queue::apply_due_writes(&mut self.streams, &mut self.accrued, epoch)?;
        self.streams_dirty |= !(result.applied.is_empty() && result.dropped.is_empty());
        Ok(result)
    }

    /// Applies due writes, then empties one queue slot.
    pub(crate) fn apply_due_and_cancel(
        &mut self,
        epoch: ChainEpoch,
        slot: Slot,
    ) -> Result<CancelResult> {
        let result =
            queue::apply_due_writes_and_cancel(&mut self.streams, &mut self.accrued, epoch, slot)?;
        self.streams_dirty = true;
        Ok(result)
    }
}

#[cfg(test)]
mod tests;
