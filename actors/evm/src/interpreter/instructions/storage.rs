use {
    crate::interpreter::{ExecutionState, StatusCode, System, U256},
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
    let value = match system.get_storage(location)? {
        Some(val) => val,
        None => U256::zero(),
    };
    state.stack.push(value);
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

    let location = state.stack.pop();
    let value = state.stack.pop();
    let opt_value = if value == U256::zero() { None } else { Some(value) };

    system.set_storage(location, opt_value)?;
    Ok(())
}
