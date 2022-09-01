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
    _state: &mut ExecutionState,
    _platform: &'r System<'r, BS, RT>,
) -> Result<(), StatusCode> {
    // Commented due to horrible borrow checker issues that stem from owning a HAMT during
    // the entire execution inside the System. The HAMT needs to be flushed and dropped when
    // create_actor, delete_actor, and send are called. All other methods taking a &mut self
    // on the Runtime should not require &mut.
    //
    // let beneficiary_addr = Address::from(state.stack.pop());
    //
    // if let Some(id_addr) = beneficiary_addr.as_id_address() {
    //     platform.rt.delete_actor(&id_addr)?;
    // } else {
    //     todo!("no support for non-ID addresses")
    // }
    //
    // Ok(())
    todo!()
}
