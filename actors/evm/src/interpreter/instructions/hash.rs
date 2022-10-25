use {
    super::memory::get_memory_region,
    crate::interpreter::{ExecutionState, StatusCode, System, U256},
    fil_actors_runtime::runtime::Runtime,
    fvm_ipld_blockstore::Blockstore,
    fvm_shared::crypto::hash::SupportedHashes,
};

pub fn keccak256<'r, BS: Blockstore, RT: Runtime<BS>>(
    system: &System<'r, BS, RT>,
    state: &mut ExecutionState,
) -> Result<(), StatusCode> {
    let index = state.stack.pop();
    let size = state.stack.pop();

    let region = get_memory_region(&mut state.memory, index, size) //
        .map_err(|_| StatusCode::InvalidMemoryAccess)?;

    let (buf, size) = system.rt.hash_64(
        SupportedHashes::Keccak256,
        if let Some(region) = region {
            &state.memory[region.offset..region.offset + region.size.get()]
        } else {
            &[]
        },
    );

    state.stack.push(U256::from_big_endian(&buf[..size]));

    Ok(())
}
