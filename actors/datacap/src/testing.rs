pub use frc46_token::token::state::StateSummary;
use fvm_ipld_blockstore::Blockstore;
use fvm_shared::address::Protocol;

use fil_actors_runtime::MessageAccumulator;

use crate::State;

/// Checks internal invariants of data cap token actor state.
pub fn check_state_invariants<BS: Blockstore>(
    state: &State,
    store: &BS,
) -> (StateSummary, MessageAccumulator) {
    let acc = MessageAccumulator::default();
    acc.require(state.governor.protocol() == Protocol::ID, "governor must be ID address");
    let (summary, errors) = state.token.check_invariants(store, 1);
    for err in errors {
        acc.add(err.to_string())
    }
    (summary, acc)
}
