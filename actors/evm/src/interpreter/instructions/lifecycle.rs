use {
    crate::interpreter::{ExecutionState, StatusCode, System, U256},
    fil_actors_runtime::runtime::Runtime,
    fvm_ipld_blockstore::Blockstore,
};

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
