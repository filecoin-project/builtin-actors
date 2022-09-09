use {
    crate::interpreter::{ExecutionState, StatusCode, System, U256},
    fil_actors_runtime::runtime::Runtime,
    fvm_ipld_blockstore::Blockstore,
};

#[inline]
pub fn sload<'r, BS: Blockstore, RT: Runtime<BS>>(
    state: &mut ExecutionState,
    platform: &'r mut System<'r, BS, RT>,
) -> Result<(), StatusCode> {
    // where?
    let location = state.stack.pop()?;

    // get from storage and place on stack
    let value = match platform.get_storage(location)? {
        Some(val) => val,
        None => U256::zero(),
    };
    state.stack.push(value)
}

#[inline]
pub fn sstore<'r, BS: Blockstore, RT: Runtime<BS>>(
    state: &mut ExecutionState,
    platform: &'r mut System<'r, BS, RT>,
) -> Result<(), StatusCode> {
    let (location, value) = state.stack.with::<2, _, _>(|args| Ok((args[1], args[0])))?;

    let opt_value = if value.is_zero() { None } else { Some(value) };

    platform.set_storage(location, opt_value)?;
    Ok(())
}
