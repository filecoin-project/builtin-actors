use {
    crate::interpreter::{ExecutionState, StatusCode, System, U256},
    fil_actors_runtime::runtime::Runtime,
    fvm_ipld_blockstore::Blockstore,
};

#[inline]
pub fn sload<'r, BS: Blockstore, RT: Runtime<BS>>(
    state: &mut ExecutionState,
    platform: &'r System<'r, BS, RT>,
) -> Result<(), StatusCode> {
    // where?
    let location = state.stack.pop();

    // get from storage and place on stack
    let value = match platform.get_storage(location)? {
        Some(val) => val,
        None => U256::zero(),
    };
    state.stack.push(value);
    Ok(())
}

#[inline]
pub fn sstore<'r, BS: Blockstore, RT: Runtime<BS>>(
    state: &mut ExecutionState,
    platform: &'r System<'r, BS, RT>,
) -> Result<(), StatusCode> {
    let location = state.stack.pop();
    let value = state.stack.pop();
    let opt_value = if value == U256::zero() { None } else { Some(value) };

    platform.set_storage(location, opt_value)?;
    Ok(())
}

#[inline]
pub fn balance<'r, BS: Blockstore, RT: Runtime<BS>>(
    _state: &mut ExecutionState,
    _platform: &'r System<'r, BS, RT>,
) -> Result<(), StatusCode> {
    todo!()
}

#[inline]
pub fn selfbalance<'r, BS: Blockstore, RT: Runtime<BS>>(
    _state: &mut ExecutionState,
    _platform: &'r System<'r, BS, RT>,
) -> Result<(), StatusCode> {
    todo!()
}

#[inline]
pub fn extcodesize<'r, BS: Blockstore, RT: Runtime<BS>>(
    _state: &mut ExecutionState,
    _platform: &'r System<'r, BS, RT>,
) -> Result<(), StatusCode> {
    todo!()
}

pub fn extcodehash<'r, BS: Blockstore, RT: Runtime<BS>>(
    _state: &mut ExecutionState,
    _platform: &'r System<'r, BS, RT>,
) -> Result<(), StatusCode> {
    todo!();
}

#[inline]
pub fn create<'r, BS: Blockstore, RT: Runtime<BS>>(
    _state: &mut ExecutionState,
    _platform: &'r System<'r, BS, RT>,
    _create2: bool,
) -> Result<(), StatusCode> {
    todo!()
}

#[inline]
pub fn selfdestruct<'r, BS: Blockstore, RT: Runtime<BS>>(
    _state: &mut ExecutionState,
    _platform: &'r System<'r, BS, RT>,
) -> Result<(), StatusCode> {
    todo!()
}
