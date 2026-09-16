//! The stream engine, one module per part of FIP-0118 2.4, operating on the persisted shapes
//! declared in [`crate::state`].
//!
//! [`Ledger`] is the state those parts share: the block behind `State.streams_root`, plus the
//! accrual rows in the state root, loaded and stored as one.

use anyhow::Result;
use fil_actors_runtime::runtime::Runtime;
use fil_actors_runtime::{ActorDowncast, ActorError, actor_error};
use fvm_ipld_encoding::{CborStore, from_slice};
use fvm_shared::econ::TokenAmount;
use fvm_shared::error::ExitCode;
use multihash_codetable::Code;
use num_traits::Zero;

use crate::state::{ExplicitDistribution, State, Stream, StreamAccrual, StreamId, StreamsState};

mod award;
mod distribution;
pub(crate) mod invariants;
mod queue;
mod weights;

pub use self::award::explicit_liability;
pub(crate) use self::award::{FullAward, plan_award};
pub(crate) use self::distribution::admit_shares;
pub(crate) use self::invariants::validate_streams_state;
pub(crate) use self::queue::{ApplyResult, QueuedCall, WriteKey};

/// Stream state that has passed the structure and accounting invariants.
///
/// Construction runs both checks so no need for a upstream re-check.
/// [`Ledger::validate_changes`] runs them once more over any changes made, so a mutation that would
/// have stored a fault causes a fail back to to the causal operation. The schedule invariants
/// don't need to be checked because claims, cancellation and `SetShares` stay usable while the
/// weight schedule is invalid; these are checked separately.
///
/// The accrual rows are part of the ledger even though they persist in the state root rather than
/// the streams block, because every liability the block describes is measured against them.
///
/// Operations mutate in place. On `Err` the ledger is unspecified and the caller discards it,
/// which every caller does: `rt.transaction` drops the state when its closure fails, and FVM
/// rollback reverts an aborted call.
///
/// Its methods are grouped by concern, one `impl` block per file: loading, storing and lookups
/// here; `admit`, `apply_due` and `cancel` in `queue.rs`; `set_shares`, `replace_address`,
/// `claim`, `remove_stream` and `replace_writer` in `distribution.rs`; `liability`, `allocate`
/// and `accrue` in `award.rs`.
#[derive(Clone)]
pub(crate) struct Ledger {
    streams: StreamsState,
    accrued: Vec<StreamAccrual>,
    /// Set by the paths that can change the streams block, which [`Ledger::store`] then writes.
    streams_dirty: bool,
}

/// An explicit stream's open period, the interval since its last `SetShares`, as its distribution
/// and the pool accrued to it (FIP-0118 2.4.4).
struct Period<'a> {
    distribution: &'a mut ExplicitDistribution,
    pool: &'a mut TokenAmount,
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

    /// Checks the invariants over any changes. We do the full set when the streams block is dirty
    /// and only the `accounting` checks when just the accrual rows were touched.
    ///
    /// `st` is used to detect whether the accrual rows are dirty.
    pub(crate) fn validate_changes(&self, st: &State) -> Result<()> {
        // --- Test builds only
        // A test can arm the fault to make one validation fail. See `arm_award_fault`.
        #[cfg(test)]
        if AWARD_FAULT.with(|armed| armed.replace(false)) {
            anyhow::bail!("test fault armed");
        }
        // ---

        let accrued_changed = st.accrued != self.accrued;
        if self.streams_dirty {
            invariants::structure(&self.streams)?;
        }
        if self.streams_dirty || accrued_changed {
            invariants::accounting(&self.streams, &self.accrued)?;
        }
        Ok(())
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
    fn period_mut(&mut self, id: StreamId) -> Option<Period<'_>> {
        let distribution = self.streams.stream_mut(id).and_then(Stream::explicit_mut)?;
        let row = self
            .accrued
            .iter_mut()
            .find(|row| row.id == id)
            .expect("accounting invariants: every explicit stream has an accrual row");
        Some(Period { distribution, pool: &mut row.amount })
    }

    /// A live explicit stream's accrual, which the accounting invariants give every one of them.
    fn accrual_mut(&mut self, id: StreamId) -> Option<&mut TokenAmount> {
        self.accrued.iter_mut().find(|row| row.id == id).map(|row| &mut row.amount)
    }

    /// Opens an accrual row for a newly registered explicit stream, keeping the rows ascending.
    fn insert_accrual(&mut self, id: StreamId) {
        let idx = self
            .accrued
            .binary_search_by_key(&id, |row| row.id)
            .expect_err("registration precondition: the stream ID holds no accrual row yet");
        self.accrued.insert(idx, StreamAccrual { id, amount: TokenAmount::zero() });
    }

    /// Closes a removed stream's accrual row and hands over its balance.
    fn take_accrual(&mut self, id: StreamId) -> Option<TokenAmount> {
        let idx = self.accrued.iter().position(|row| row.id == id)?;
        Some(self.accrued.remove(idx).amount)
    }
}

#[cfg(test)]
thread_local! {
    /// Armed by the test that needs a planned award to fail validation.
    /// Consumed by the next `validate_changes`.
    static AWARD_FAULT: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

#[cfg(test)]
impl Ledger {
    /// Adds an accrual row not belonging to a real stream, this should be rejected by accounting
    /// invariants.
    pub(crate) fn corrupt(&mut self) {
        self.accrued.push(StreamAccrual { id: StreamId::MAX, amount: TokenAmount::zero() });
    }

    /// Makes the next `validate_changes` fail.
    pub(crate) fn arm_award_fault() {
        AWARD_FAULT.with(|armed| armed.set(true));
    }
}

#[cfg(test)]
mod tests;
