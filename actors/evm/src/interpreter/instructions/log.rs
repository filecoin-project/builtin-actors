use crate::interpreter::instructions::memory::get_memory_region;
use {
    crate::interpreter::{ExecutionState, StatusCode, System},
    fil_actors_runtime::runtime::Runtime,
};

#[inline]
pub fn log(
    state: &mut ExecutionState,
    _system: &System<impl Runtime>,
    num_topics: usize,
) -> Result<(), StatusCode> {
    // Handle the payload.
    let mem_index = state.stack.pop();
    let size = state.stack.pop();
    let payload = get_memory_region(&mut state.memory, mem_index, size)
        .map_err(|_| StatusCode::InvalidMemoryAccess)?;

    // Extract the topics.
    let topics: Vec<_> = (0..num_topics).map(|_| state.stack.pop()).collect();
    for _ in 0..num_topics {
        state.stack.pop();
    }

    Ok(())
}
