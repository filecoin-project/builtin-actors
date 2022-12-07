use {
    crate::interpreter::{ExecutionState, StatusCode, System, U256},
    fil_actors_runtime::runtime::Runtime,
};

#[inline]
pub fn sload(
    _state: &mut ExecutionState,
    system: &mut System<impl Runtime>,
    location: U256,
) -> Result<U256, StatusCode> {
    // get from storage and place on stack
    system.get_storage(location)
}

#[inline]
pub fn sstore(
    _state: &mut ExecutionState,
    system: &mut System<impl Runtime>,
    key: U256,
    value: U256,
) -> Result<(), StatusCode> {
    if system.readonly {
        return Err(StatusCode::StaticModeViolation);
    }

    system.set_storage(key, value)
}
