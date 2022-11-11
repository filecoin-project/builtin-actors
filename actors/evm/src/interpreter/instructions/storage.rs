use {
    crate::interpreter::{ExecutionState, StatusCode, System},
    fil_actors_runtime::runtime::Runtime,
};

#[inline]
pub fn sload(
    state: &mut ExecutionState,
    system: &mut System<impl Runtime>,
) -> Result<(), StatusCode> {
    // where?
    let location = state.stack.pop();

    // get from storage and place on stack
    state.stack.push(system.get_storage(location)?);
    Ok(())
}

#[inline]
pub fn sstore(
    state: &mut ExecutionState,
    system: &mut System<impl Runtime>,
) -> Result<(), StatusCode> {
    if system.readonly {
        return Err(StatusCode::StaticModeViolation);
    }

    let key = state.stack.pop();
    let value = state.stack.pop();

    system.set_storage(key, value)?;
    Ok(())
}
