use {
    super::memory::{get_memory_region, num_words},
    crate::interp::output::StatusCode,
    crate::interp::stack::Stack,
    crate::interp::CallKind,
    crate::interp::ExecutionState,
    crate::interp::System,
    crate::interp::U256,
    fvm_ipld_blockstore::Blockstore,
};

#[inline]
pub fn calldataload(state: &mut ExecutionState) {
    let index = state.stack.pop();
    let input_len = state.message.input_data.len();

    state.stack.push({
        if index > U256::from(input_len) {
            U256::zero()
        } else {
            let index_usize = index.as_usize();
            let end = core::cmp::min(index_usize + 32, input_len);

            let mut data = [0; 32];
            data[..end - index_usize].copy_from_slice(&state.message.input_data[index_usize..end]);

            U256::from_big_endian(&data)
        }
    });
}

#[inline]
pub fn calldatasize(state: &mut ExecutionState) {
    state.stack.push(u128::try_from(state.message.input_data.len()).unwrap().into());
}

#[inline]
pub fn calldatacopy(state: &mut ExecutionState) -> Result<(), StatusCode> {
    let mem_index = state.stack.pop();
    let input_index = state.stack.pop();
    let size = state.stack.pop();

    let region = get_memory_region(state, mem_index, size).map_err(|_| StatusCode::OutOfGas)?;

    if let Some(region) = &region {
        let copy_cost = num_words(region.size.get()) * 3;
        state.gas_left -= copy_cost as i64;
        if state.gas_left < 0 {
            return Err(StatusCode::OutOfGas);
        }

        let input_len = U256::from(state.message.input_data.len());
        let src = core::cmp::min(input_len, input_index);
        let copy_size = core::cmp::min(size, input_len - src).as_usize();
        let src = src.as_usize();

        if copy_size > 0 {
            state.memory[region.offset..region.offset + copy_size]
                .copy_from_slice(&state.message.input_data[src..src + copy_size]);
        }

        if region.size.get() > copy_size {
            state.memory[region.offset + copy_size..region.offset + region.size.get()].fill(0);
        }
    }

    Ok(())
}

#[inline]
pub fn codesize(stack: &mut Stack, code: &[u8]) {
    stack.push(U256::from(code.len()))
}

#[inline]
pub fn codecopy(state: &mut ExecutionState, code: &[u8]) -> Result<(), StatusCode> {
    let mem_index = state.stack.pop();
    let input_index = state.stack.pop();
    let size = state.stack.pop();

    let region = get_memory_region(state, mem_index, size).map_err(|_| StatusCode::OutOfGas)?;

    if let Some(region) = region {
        let src = core::cmp::min(U256::from(code.len()), input_index).as_usize();
        let copy_size = core::cmp::min(region.size.get(), code.len() - src);

        let copy_cost = num_words(region.size.get()) * 3;
        state.gas_left -= copy_cost as i64;
        if state.gas_left < 0 {
            return Err(StatusCode::OutOfGas);
        }

        if copy_size > 0 {
            state.memory[region.offset..region.offset + copy_size]
                .copy_from_slice(&code[src..src + copy_size]);
        }

        if region.size.get() > copy_size {
            state.memory[region.offset + copy_size..region.offset + region.size.get()].fill(0);
        }
    }

    Ok(())
}

#[inline]
pub fn call<'r, BS: Blockstore>(
    _state: &mut ExecutionState,
    _platform: &'r System<'r, BS>,
    _kind: CallKind,
    _is_static: bool,
) -> Result<(), StatusCode> {
    todo!();
}
