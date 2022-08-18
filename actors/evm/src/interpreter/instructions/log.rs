use {
    crate::interpreter::{ExecutionState, StatusCode, System},
    fil_actors_runtime::runtime::Runtime,
    fvm_ipld_blockstore::Blockstore,
};

#[inline]
pub fn log<'r, BS: Blockstore, RT: Runtime<BS>>(
    _state: &mut ExecutionState,
    _platform: &'r System<'r, BS, RT>,
    _num_topics: usize,
) -> Result<(), StatusCode> {
    todo!()
}
