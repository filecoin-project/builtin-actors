use fil_actors_evm_shared::uints::U256;
use fil_actors_runtime::ActorError;

use {
    crate::interpreter::{ExecutionState, System},
    fil_actors_runtime::runtime::Runtime,
};

#[inline]
pub fn sload(
    _state: &mut ExecutionState,
    system: &mut System<impl Runtime>,
    location: U256,
) -> Result<U256, ActorError> {
    // get from storage and place on stack
    system.get_storage(location)
}

#[inline]
pub fn sstore(
    _state: &mut ExecutionState,
    system: &mut System<impl Runtime>,
    key: U256,
    value: U256,
) -> Result<(), ActorError> {
    if system.readonly {
        return Err(ActorError::read_only("store called while read-only".into()));
    }

    system.set_storage(key, value)
}
