use {
    super::memory::get_memory_region, crate::interpreter::output::StatusCode,
    crate::interpreter::stack::Stack, crate::interpreter::Bytecode,
    crate::interpreter::ExecutionState, crate::interpreter::U256,
};

#[inline]
pub fn ret(state: &mut ExecutionState) -> Result<(), StatusCode> {
    state.stack.with2(|offset, size| {
        if let Some(region) = super::memory::get_memory_region(&mut state.memory, offset, size)
            .map_err(|_| StatusCode::InvalidMemoryAccess)?
        {
            state.output_data =
                state.memory[region.offset..region.offset + region.size.get()].to_vec().into();
        }

        Ok(())
    })
}

#[inline]
pub fn returndatasize(state: &mut ExecutionState) -> Result<(), StatusCode> {
    state.stack.push(U256::from(state.return_data.len()))
}

#[inline]
pub fn returndatacopy(state: &mut ExecutionState) -> Result<(), StatusCode> {
    state.stack.with3(|mem_index, input_index, size| {
        let region = get_memory_region(&mut state.memory, mem_index, size)
            .map_err(|_| StatusCode::InvalidMemoryAccess)?;

        let src = input_index.as_usize();
        if src > state.return_data.len() {
            return Err(StatusCode::InvalidMemoryAccess);
        }

        if src + region.as_ref().map(|r| r.size.get()).unwrap_or(0) > state.return_data.len() {
            return Err(StatusCode::InvalidMemoryAccess);
        }

        if let Some(region) = region {
            state.memory[region.offset..region.offset + region.size.get()]
                .copy_from_slice(&state.return_data[src..src + region.size.get()]);
        }

        Ok(())
    })
}

#[inline]
pub fn gas(_state: &mut ExecutionState) -> Result<(), StatusCode> {
    todo!()
}

#[inline]
pub fn pc(stack: &mut Stack, pc: usize) -> Result<(), StatusCode> {
    stack.push(U256::from(pc))
}

#[inline]
fn jump_target(dest: &U256, bytecode: &Bytecode) -> Result<usize, StatusCode> {
    let dest = dest.as_usize(); // XXX as_usize can panic if it doesn't fit
    if !bytecode.valid_jump_destination(dest) {
        return Err(StatusCode::BadJumpDestination);
    }
    Ok(dest)
}

#[inline]
pub fn jump(stack: &mut Stack, bytecode: &Bytecode) -> Result<usize, StatusCode> {
    let dest = stack.pop()?;
    jump_target(&dest, bytecode)
}

#[inline]
pub fn jumpi(stack: &mut Stack, bytecode: &Bytecode) -> Result<Option<usize>, StatusCode> {
    stack.with2(|dest, cond| {
        if !cond.is_zero() {
            let dest = jump_target(dest, bytecode)?;
            Ok(Some(dest))
        } else {
            Ok(None)
        }
    })
}
