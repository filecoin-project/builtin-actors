use {
    crate::interp::ExecutionState, crate::interp::StatusCode, crate::interp::System,
    fvm_ipld_blockstore::Blockstore,
};

#[inline]
pub fn log<'r, BS: Blockstore>(
    _state: &mut ExecutionState,
    _platform: &'r System<'r, BS>,
    _num_topics: usize,
) -> Result<(), StatusCode> {
    todo!()
}
