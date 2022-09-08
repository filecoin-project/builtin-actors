use crate::interpreter::address::Address;
use {
    crate::interpreter::{ExecutionState, StatusCode, System},
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
    state: &mut ExecutionState,
    _system: &'r mut System<'r, BS, RT>,
) -> Result<(), StatusCode> {
    let beneficiary_addr = Address::try_from(state.stack.pop())?;
    let id_addr = beneficiary_addr.as_id_address().expect("no support for non-ID addresses yet");
    state.selfdestroyed = Some(id_addr);
    Ok(())
}
