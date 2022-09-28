use fvm_ipld_blockstore::Blockstore;
use fvm_shared::address::Protocol;

use fil_actors_runtime::MessageAccumulator;

use crate::State;

pub struct StateSummary {}

/// Checks internal invariants of data cap token actor state.
pub fn check_state_invariants<BS: Blockstore>(
    state: &State,
    store: &BS,
) -> (StateSummary, MessageAccumulator) {
    let acc = MessageAccumulator::default();
    acc.require(state.governor.protocol() == Protocol::ID, "registry must be ID address");
    let r = state.token.check_invariants(store);
    if let Err(e) = r {
        acc.add(e.to_string());
    }

    (StateSummary {}, acc)
}
