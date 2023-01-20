use fil_actors_runtime::ActorError;

use {
    super::memory::get_memory_region,
    crate::interpreter::{ExecutionState, System, U256},
    fil_actors_runtime::runtime::Runtime,
    fvm_shared::crypto::hash::SupportedHashes,
};

pub fn keccak256(
    state: &mut ExecutionState,
    system: &System<impl Runtime>,
    index: U256,
    size: U256,
) -> Result<U256, ActorError> {
    let region = get_memory_region(&mut state.memory, index, size)?;

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
