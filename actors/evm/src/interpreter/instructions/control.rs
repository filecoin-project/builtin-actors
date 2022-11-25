use {
    super::memory::get_memory_region,
    crate::interpreter::output::StatusCode,
    crate::interpreter::Bytecode,
    crate::interpreter::{ExecutionState, System, U256},
    fil_actors_runtime::runtime::Runtime,
};

#[inline]
pub fn output(
    state: &mut ExecutionState,
    _system: &System<impl Runtime>,
    offset: U256,
    size: U256,
) -> Result<(), StatusCode> {
    if let Some(region) = super::memory::get_memory_region(&mut state.memory, offset, size)
        .map_err(|_| StatusCode::InvalidMemoryAccess)?
    {
        state.output_data =
            state.memory[region.offset..region.offset + region.size.get()].to_vec().into();
    }

    Ok(())
}

#[inline]
pub fn returndatasize(
    state: &mut ExecutionState,
    _system: &System<impl Runtime>,
) -> Result<U256, StatusCode> {
    Ok(U256::from(state.return_data.len()))
}

#[inline]
pub fn returndatacopy(
    state: &mut ExecutionState,
    _system: &System<impl Runtime>,
    mem_index: U256,
    input_index: U256,
    size: U256,
) -> Result<(), StatusCode> {
    let region = get_memory_region(&mut state.memory, mem_index, size)
        .map_err(|_| StatusCode::InvalidMemoryAccess)?;

    if input_index > U256::from(state.return_data.len()) {
        return Err(StatusCode::InvalidMemoryAccess);
    }
    let src = input_index.as_usize();

    if src + region.as_ref().map(|r| r.size.get()).unwrap_or(0) > state.return_data.len() {
        return Err(StatusCode::InvalidMemoryAccess);
    }

    if let Some(region) = region {
        state.memory[region.offset..region.offset + region.size.get()]
            .copy_from_slice(&state.return_data[src..src + region.size.get()]);
    }

    Ok(())
}

#[inline]
pub fn jump(bytecode: &Bytecode, dest: U256) -> Result<Option<usize>, StatusCode> {
    let dst = dest.as_usize();
    if !bytecode.valid_jump_destination(dst) {
        return Err(StatusCode::BadJumpDestination);
    }
    Ok(Some(dst))
}

#[inline]
pub fn jumpi(bytecode: &Bytecode, dest: U256, test: U256) -> Result<Option<usize>, StatusCode> {
    if !test.is_zero() {
        let dst = dest.as_usize();
        if !bytecode.valid_jump_destination(dst) {
            return Err(StatusCode::BadJumpDestination);
        }
        Ok(Some(dst))
    } else {
        Ok(None)
    }
}
