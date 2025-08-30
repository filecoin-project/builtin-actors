use fil_actors_runtime::MessageAccumulator;
use fvm_shared::address::Address;

use crate::State;

pub struct StateSummary {
    pub validator: Address,
}

pub fn check_state_invariants(
    state: &State,
    _id_address: &Address,
) -> (StateSummary, MessageAccumulator) {
    let acc = MessageAccumulator::default();

    // TODO: Add invariants
    // check allocated sectors bitfield
    // check verifier actor exists

    (StateSummary { validator: state.validator }, acc)
} 