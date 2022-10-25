use {
    crate::interpreter::{ExecutionState, StatusCode, System},
    fil_actors_runtime::runtime::Runtime,
    fvm_ipld_blockstore::Blockstore,
};

#[cfg(debug_assertions)]
pub fn log<'r, BS: Blockstore, RT: Runtime<BS>>(
    _state: &mut ExecutionState,
    _system: &'r System<'r, BS, RT>,
    _num_topics: usize,
) -> Result<(), StatusCode> {
    todo!("unimplemented");
}

#[cfg(not(debug_assertions))]
#[inline]
pub fn log<'r, BS: Blockstore, RT: Runtime<BS>>(
    state: &mut ExecutionState,
    _system: &'r System<'r, BS, RT>,
    num_topics: usize,
) -> Result<(), StatusCode> {
    // TODO: Right now, we just drop everything. But we implement this in production anyways so
    // things work.
    for _ in 0..num_topics {
        state.stack.pop();
    }
    Ok(())
}
