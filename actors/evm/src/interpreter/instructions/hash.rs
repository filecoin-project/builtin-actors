use {
    super::memory::get_memory_region,
    crate::interpreter::{ExecutionState, StatusCode, System, U256},
    fil_actors_runtime::runtime::Runtime,
    fvm_shared::crypto::hash::SupportedHashes,
};

pub fn keccak256(
    state: &mut ExecutionState,
    system: &System<impl Runtime>,
    index: U256,
    size: U256,
) -> Result<U256, StatusCode> {
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

    Ok(U256::from_big_endian(&buf[..size]))
}
