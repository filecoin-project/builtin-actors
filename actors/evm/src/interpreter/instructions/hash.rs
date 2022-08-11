use {
    super::memory::{get_memory_region, num_words},
    crate::interpreter::ExecutionState,
    crate::interpreter::StatusCode,
    crate::interpreter::U256,
    sha3::{Digest, Keccak256},
};

pub fn keccak256(state: &mut ExecutionState) -> Result<(), StatusCode> {
    let index = state.stack.pop();
    let size = state.stack.pop();

    let region = get_memory_region(state, index, size) //
        .map_err(|_| StatusCode::OutOfGas)?;

    state.stack.push(U256::from_big_endian(&*Keccak256::digest(if let Some(region) = region {
        let w = num_words(region.size.get());
        let cost = w * 6;
        state.gas_left -= cost as i64;
        if state.gas_left < 0 {
            return Err(StatusCode::OutOfGas);
        }

        &state.memory[region.offset..region.offset + region.size.get()]
    } else {
        &[]
    })));

    Ok(())
}
