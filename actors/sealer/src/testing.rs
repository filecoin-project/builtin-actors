use fil_actors_runtime::MessageAccumulator;
use fvm_shared::address::Address;

use crate::State;

pub struct StateSummary {
    pub id_addr: Address,
}

pub fn check_state_invariants(
    state: &State,
    _id_address: &Address,
) -> (StateSummary, MessageAccumulator) {
    let acc = MessageAccumulator::default();
    // TODO: Add invariants as needed
    (StateSummary { id_addr: state.id_addr }, acc)
} 