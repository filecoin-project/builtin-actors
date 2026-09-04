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
use fvm_shared::econ::TokenAmount;
use fvm_shared::error::ExitCode;
use multihash_codetable::Code;
use num_traits::Zero;

use crate::State;

mod award;
mod distribution;
pub(crate) mod invariants;
mod queue;
mod weights;

pub use self::award::explicit_liability;
pub(crate) use self::award::{FullAward, plan_award};
pub(crate) use self::distribution::normalize_shares;
pub use self::distribution::{
    DistributionInit, ExplicitDistribution, RecipientAmount, RecipientShare, RecipientTable,
};
pub(crate) use self::invariants::validate_streams_state;
pub(crate) use self::queue::{ApplyResult, QueuedCall, Slot};
pub use self::queue::{
    PendingWrite, PendingWriteOp, RegisterStreamPayload, SetDistributionPayload,
};
pub use self::weights::{WeightRecord, WeightRecordUpdate, WeightRecordsPayload};

pub type StreamId = u64;

// These bound the work an award or a mutation can do, and raising any of them takes a FIP and a
// network upgrade that reshapes the affected structures for the new scale (FIP-0118 2.4.9).

/// The fixed point scale for weights and shares (FIP-0118 2.4.1(4)). At 10^18 the percentages the
/// FIP is written in are exact integers, and a wire share map sums to exactly this.
pub const DENOM: u64 = 1_000_000_000_000_000_000;
/// Streams the schedule can carry, counting the implicit consensus stream.
pub const MAX_STREAMS: usize = 8;
/// Recipients in one share map, and wallets in one `Claim` batch.
pub const MAX_RECIPIENTS: usize = 64;
/// Payable rows one live stream carries, which covers the union of an outgoing share map with an
/// incoming one. The cap is per stream, so a hostile writer fills only its own stream's table, and
/// one full `Claim` batch drains half of it.
pub const MAX_PAYABLE_ROWS_PER_STREAM: usize = 2 * MAX_RECIPIENTS;
/// Payable rows across every tombstone. Each pending removal reserves at least `MAX_RECIPIENTS` of
/// it, so at most four removals are ever in flight together.
pub const MAX_TOMBSTONE_ROWS: usize = 256;
/// Registration, removal and writer change, the three operations one stream can have queued.
const STREAM_SCOPED_PENDING_OPS: usize = 3;
/// `SetWeightRecords` and `StepWeightRecords`, which act on the whole schedule.
const SCHEDULE_WIDE_PENDING_OPS: usize = 2;
/// Entries in the pending-write queue, which is the slot space of every per-stream operation
/// across a full stream table plus the two schedule-wide slots. The queue enforces it as a plain
/// length check rather than deriving it from the slots in use.
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
///
/// A removed stream leaves a tombstone under its own ID holding the rows it had not paid, so a
/// live ID and a tombstoned ID never name the same stream at once. A `RegisterStream` for an ID
/// that is live, tombstoned or already queued for registration is rejected at admission by
/// `ensure_stream_id_available`, and one whose ID has become live or tombstoned since strands at
/// application as `Stranded::StreamIdInUse`. An ID therefore comes back into use only once its
/// tombstone is gone, which is when its last row is claimed, and a removal that leaves nothing
/// unpaid files no tombstone at all.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize_tuple, Deserialize_tuple)]
pub struct StreamsState {
    /// Ordered by stream ID.
    pub streams: Vec<Stream>,
    /// Ordered by stream ID, disjoint from the live streams.
    pub tombstones: Vec<Tombstone>,
    /// Ordered by effective epoch; equal epochs retain queue position.
    pub pending_writes: Vec<PendingWrite>,
}

/// Lookups by stream ID. The tables are bounded, so a linear scan is fine.
impl StreamsState {
    pub(crate) fn stream(&self, id: StreamId) -> Option<&Stream> {
        self.streams.iter().find(|stream| stream.id == id)
    }

    pub(crate) fn stream_mut(&mut self, id: StreamId) -> Option<&mut Stream> {
        self.streams.iter_mut().find(|stream| stream.id == id)
    }

    /// The stored writer and recipient tables of a live explicit stream.
    pub(crate) fn explicit(&self, id: StreamId) -> Option<&ExplicitDistribution> {
        self.stream(id).and_then(Stream::explicit)
    }

    pub(super) fn explicit_mut(&mut self, id: StreamId) -> Option<&mut ExplicitDistribution> {
        self.stream_mut(id).and_then(Stream::explicit_mut)
    }

    pub(crate) fn has_stream(&self, id: StreamId) -> bool {
        self.stream(id).is_some()
    }

    /// Files a live stream, keeping the table ascending by stream ID.
    pub(super) fn insert_stream(&mut self, stream: Stream) {
        let idx = self
            .streams
            .binary_search_by_key(&stream.id, |live| live.id)
            .expect_err("registration precondition: the stream ID is not live");
        self.streams.insert(idx, stream);
    }

    /// Removes the live stream with this ID and hands it over.
    pub(super) fn take_stream(&mut self, id: StreamId) -> Option<Stream> {
        let idx = self.streams.iter().position(|stream| stream.id == id)?;
        Some(self.streams.remove(idx))
    }

    pub(super) fn tombstone_mut(&mut self, id: StreamId) -> Option<&mut Tombstone> {
        self.tombstones.iter_mut().find(|tombstone| tombstone.id == id)
    }

    pub(crate) fn has_tombstone(&self, id: StreamId) -> bool {
        self.tombstones.iter().any(|tombstone| tombstone.id == id)
    }

    /// Files a removed stream's unpaid rows, keeping the tombstones ascending by stream ID.
    pub(super) fn insert_tombstone(&mut self, id: StreamId, payable: RecipientTable) {
        let idx = self
            .tombstones
            .binary_search_by_key(&id, |tombstone| tombstone.id)
            .expect_err("structure invariants: live and tombstoned stream IDs are disjoint");
        self.tombstones.insert(idx, Tombstone { id, payable });
    }

    /// Removes a drained tombstone.
    pub(super) fn take_tombstone(&mut self, id: StreamId) -> Option<Tombstone> {
        let idx = self.tombstones.iter().position(|tombstone| tombstone.id == id)?;
        Some(self.tombstones.remove(idx))
    }
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
///
/// Operations mutate in place. On `Err` the ledger is unspecified and the caller discards it,
/// which every caller does: `rt.transaction` drops the state when its closure fails, and FVM
/// rollback reverts an aborted call.
#[derive(Clone)]
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

    /// A live explicit stream's distribution together with the accrual that it pays from.
    ///
    /// The accounting invariants pair the two so this becomes `None` only when `id` doesn't
    /// identify a live explicit stream or it identifies the implicit one.
    /// A fold also needs both halves at once and they live in different structures, so this is the
    /// one lookup that borrows the pair together.
    fn explicit_and_accrual_mut(
        &mut self,
        id: StreamId,
    ) -> Option<(&mut ExplicitDistribution, &mut TokenAmount)> {
        let distribution = self.streams.explicit_mut(id)?;
        let row = self
            .accrued
            .iter_mut()
            .find(|row| row.id == id)
            .expect("accounting invariants: every explicit stream has an accrual row");
        Some((distribution, &mut row.amount))
    }

    /// A live explicit stream's accrual, which the accounting invariants give every one of them.
    fn accrual_mut(&mut self, id: StreamId) -> Option<&mut TokenAmount> {
        self.accrued.iter_mut().find(|row| row.id == id).map(|row| &mut row.amount)
    }

    /// Opens an accrual row for a newly registered explicit stream, keeping the rows ascending.
    fn insert_accrual(&mut self, id: StreamId) {
        let idx = self.accrued.binary_search_by_key(&id, |row| row.id).expect_err(
            "accounting invariants: an accrual row belongs to one live explicit stream",
        );
        self.accrued.insert(idx, StreamAccrual { id, amount: TokenAmount::zero() });
    }

    /// Closes a removed stream's accrual row and hands over its balance.
    fn take_accrual(&mut self, id: StreamId) -> Option<TokenAmount> {
        let idx = self.accrued.iter().position(|row| row.id == id)?;
        Some(self.accrued.remove(idx).amount)
    }
}

#[cfg(test)]
mod tests;
