use fvm_ipld_blockstore::Blockstore;
use fvm_shared::address::Protocol;

use fil_actors_runtime::MessageAccumulator;

use crate::State;

pub struct StateSummary {}

/// Checks internal invariants of verified registry state.
pub fn check_state_invariants<BS: Blockstore>(
    state: &State,
    _store: &BS,
) -> (StateSummary, MessageAccumulator) {
    let acc = MessageAccumulator::default();
    acc.require(state.registry.protocol() == Protocol::ID, "registry must be ID address");
    // TODO: Check invariants in token state.

    (StateSummary {}, acc)
}
