use {
    super::memory::{get_memory_region, num_words},
    crate::interp::output::StatusCode,
    crate::interp::stack::Stack,
    crate::interp::Bytecode,
    crate::interp::ExecutionState,
    crate::interp::U256,
};

#[inline]
pub fn ret(state: &mut ExecutionState) -> Result<(), StatusCode> {
    let offset = *state.stack.get(0);
    let size = *state.stack.get(1);

    if let Some(region) =
        super::memory::get_memory_region(state, offset, size).map_err(|_| StatusCode::OutOfGas)?
    {
        state.output_data =
            state.memory[region.offset..region.offset + region.size.get()].to_vec().into();
    }

    Ok(())
}

#[inline]
pub fn returndatasize(state: &mut ExecutionState) {
    state.stack.push(U256::from(state.return_data.len()));
}

#[inline]
pub fn returndatacopy(state: &mut ExecutionState) -> Result<(), StatusCode> {
    let mem_index = state.stack.pop();
    let input_index = state.stack.pop();
    let size = state.stack.pop();

    let region = get_memory_region(state, mem_index, size).map_err(|_| StatusCode::OutOfGas)?;

    if input_index > U256::from(state.return_data.len()) {
        return Err(StatusCode::InvalidMemoryAccess);
    }
    let src = input_index.as_usize();

    if src + region.as_ref().map(|r| r.size.get()).unwrap_or(0) > state.return_data.len() {
        return Err(StatusCode::InvalidMemoryAccess);
    }

    if let Some(region) = region {
        let copy_cost = num_words(region.size.get()) * 3;
        state.gas_left -= copy_cost as i64;
        if state.gas_left < 0 {
            return Err(StatusCode::OutOfGas);
        }

        state.memory[region.offset..region.offset + region.size.get()]
            .copy_from_slice(&state.return_data[src..src + region.size.get()]);
    }

    Ok(())
}

#[inline]
pub fn gas(state: &mut ExecutionState) {
    state.stack.push(U256::from(state.gas_left))
}

#[inline]
pub fn pc(stack: &mut Stack, pc: usize) {
    stack.push(U256::from(pc))
}

#[inline]
pub fn jump(stack: &mut Stack, bytecode: &Bytecode) -> Result<usize, StatusCode> {
    let dst = stack.pop().as_usize();
    if !bytecode.valid_jump_destination(dst) {
        return Err(StatusCode::BadJumpDestination);
    }
    Ok(dst)
}

#[inline]
pub fn jumpi(stack: &mut Stack, bytecode: &Bytecode) -> Result<Option<usize>, StatusCode> {
    if *stack.get(1) != U256::zero() {
        let ret = Ok(Some(jump(stack, bytecode)?));
        stack.pop();
        ret
    } else {
        stack.pop();
        stack.pop();

        Ok(None)
    }
}
