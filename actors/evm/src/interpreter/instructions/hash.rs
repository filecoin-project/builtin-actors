use {
    super::memory::get_memory_region,
    crate::interpreter::ExecutionState,
    crate::interpreter::StatusCode,
    crate::interpreter::U256,
    sha3::{Digest, Keccak256},
};

pub fn keccak256(state: &mut ExecutionState) -> Result<(), StatusCode> {
    let result = state.stack.with2(|index, size| {
        let region = get_memory_region(&mut state.memory, index, size) //
            .map_err(|_| StatusCode::InvalidMemoryAccess)?;

        Ok(U256::from_big_endian(&*Keccak256::digest(if let Some(region) = region {
            &state.memory[region.offset..region.offset + region.size.get()]
        } else {
            &[]
        })))
    })?;
    state.stack.push(result)
}
