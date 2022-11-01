use {
    crate::interpreter::{ExecutionState, StatusCode, System},
    fil_actors_runtime::runtime::Runtime,
};

#[cfg(debug_assertions)]
pub fn log(
    _state: &mut ExecutionState,
    _system: &System<impl Runtime>,
    _num_topics: usize,
) -> Result<(), StatusCode> {
    todo!("unimplemented");
}

#[cfg(not(debug_assertions))]
#[inline]
pub fn log(
    state: &mut ExecutionState,
    _system: &System<impl Runtime>,
    num_topics: usize,
) -> Result<(), StatusCode> {
    // TODO: Right now, we just drop everything. But we implement this in production anyways so
    // things work.
    for _ in 0..num_topics {
        state.stack.pop();
    }
    Ok(())
}
