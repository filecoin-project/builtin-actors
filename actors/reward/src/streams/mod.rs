//! The stream engine, one module per part of FIP-0118 2.4, operating on the persisted shapes
//! declared in [`crate::state`].
//!
//! [`Ledger`] is the state those parts share: the block behind `State.streams_root`, loaded,
//! validated and stored as one.

use anyhow::Result;
use fil_actors_runtime::runtime::Runtime;
use fil_actors_runtime::{ActorDowncast, ActorError, actor_error};
use fvm_ipld_encoding::{CborStore, from_slice};
use fvm_shared::error::ExitCode;
use multihash_codetable::Code;

use crate::state::{State, StreamsState};

mod award;
mod distribution;
pub(crate) mod invariants;
mod queue;
mod weights;

pub(crate) use self::award::{FullAward, Payout, plan_award};
pub(crate) use self::distribution::admit_shares;
pub(crate) use self::invariants::validate_streams_state;
pub(crate) use self::queue::{ApplyResult, QueuedCall, Slot};

/// Stream state that has passed the structure invariants.
///
/// Construction is the only place they run, and it is the only way to obtain one, so an operation
/// cannot be handed state it needs to re-check. The schedule invariants are deliberately not
/// checked here: cancellation and `SetShares` stay usable while the weight schedule is invalid
/// (FIP-0118 2.4.8), so they are checked separately.
///
/// Operations mutate in place. On `Err` the ledger is unspecified and the caller discards it,
/// which every caller does: `rt.transaction` drops the state when its closure fails, and FVM
/// rollback reverts an aborted call.
#[derive(Clone)]
pub(crate) struct Ledger {
    streams: StreamsState,
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
        Ledger::checked(streams).map_err(|e| {
            e.downcast_default(ExitCode::USR_ILLEGAL_STATE, "invalid persisted stream state")
        })
    }

    /// Decodes and validates the streams block for the award, whose every failure pays gas only.
    pub(crate) fn decode_for_award(bytes: &[u8]) -> Result<Ledger> {
        Ledger::checked(from_slice(bytes)?)
    }

    fn checked(streams: StreamsState) -> Result<Ledger> {
        invariants::structure(&streams)?;
        Ok(Ledger { streams, streams_dirty: false })
    }

    /// Writes the streams block when this ledger has touched it.
    pub(crate) fn store(&self, rt: &impl Runtime, st: &mut State) -> Result<(), ActorError> {
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
}

#[cfg(test)]
mod tests;
