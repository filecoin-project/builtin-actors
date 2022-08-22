use {
    super::memory::get_memory_region,
    crate::interpreter::instructions::memory::MemoryRegion,
    crate::interpreter::output::StatusCode,
    crate::interpreter::precompiles,
    crate::interpreter::stack::Stack,
    crate::interpreter::ExecutionState,
    crate::interpreter::System,
    crate::interpreter::{H160, U256},
    fil_actors_runtime::runtime::Runtime,
    fvm_ipld_blockstore::Blockstore,
};

/// The kind of call-like instruction.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CallKind {
    Call,
    DelegateCall,
    CallCode,
    Create,
    Create2 { salt: U256 },
}

#[inline]
pub fn calldataload(state: &mut ExecutionState) {
    let index = state.stack.pop();
    let input_len = state.input_data.len();

    state.stack.push({
        if index > U256::from(input_len) {
            U256::zero()
        } else {
            let index_usize = index.as_usize();
            let end = core::cmp::min(index_usize + 32, input_len);

            let mut data = [0; 32];
            data[..end - index_usize].copy_from_slice(&state.input_data[index_usize..end]);

            U256::from_big_endian(&data)
        }
    });
}

#[inline]
pub fn calldatasize(state: &mut ExecutionState) {
    state.stack.push(u128::try_from(state.input_data.len()).unwrap().into());
}

#[inline]
pub fn calldatacopy(state: &mut ExecutionState) -> Result<(), StatusCode> {
    let mem_index = state.stack.pop();
    let input_index = state.stack.pop();
    let size = state.stack.pop();

    let region = get_memory_region(&mut state.memory, mem_index, size).map_err(|_| StatusCode::OutOfGas)?;

    if let Some(region) = &region {
        let input_len = U256::from(state.input_data.len());
        let src = core::cmp::min(input_len, input_index);
        let copy_size = core::cmp::min(size, input_len - src).as_usize();
        let src = src.as_usize();

        if copy_size > 0 {
            state.memory[region.offset..region.offset + copy_size]
                .copy_from_slice(&state.input_data[src..src + copy_size]);
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

    let region = get_memory_region(&mut state.memory, mem_index, size).map_err(|_| StatusCode::OutOfGas)?;

    if let Some(region) = region {
        let src = core::cmp::min(U256::from(code.len()), input_index).as_usize();
        let copy_size = core::cmp::min(region.size.get(), code.len() - src);

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
pub fn call<'r, BS: Blockstore, RT: Runtime<BS>>(
    state: &mut ExecutionState,
    _platform: &'r System<'r, BS, RT>,
    kind: CallKind,
    is_static: bool,
) -> Result<(), StatusCode> {
    let ExecutionState { stack, memory, .. } = state;

    let gas = stack.pop();
    let dst: H160 = crate::interpreter::uints::_u256_to_address(stack.pop());
    let value = if is_static || matches!(kind, CallKind::DelegateCall) {
        U256::zero()
    } else {
        stack.pop()
    };
    let has_value = !value.is_zero();
    let input_offset = stack.pop();
    let input_size = stack.pop();
    let output_offset = stack.pop();
    let output_size = stack.pop();

    stack.push(U256::zero()); // Assume failure. TODO wha

    // TODO Errs
    let input_region = get_memory_region(memory, input_offset, input_size).unwrap();
    let output_region = get_memory_region(memory, output_offset, output_size).unwrap();

    let output = {
        // ref to memory is dropped after calling so we can mutate it on output later
        let input_data = input_region
            .map(|MemoryRegion { offset, size }| &memory[offset..offset + size.get()])
            .unwrap_or_default();

        let output = if precompiles::is_precompile(&dst) {
            precompiles::call_precompile(dst, &input_data, gas.as_u64())
        } else {
            todo!()
        };

        output.unwrap().output
    };

    let output_data = output_region
        .map(|MemoryRegion { offset, size }| {
            &mut memory[offset..offset + size.get()] // would like to use get for this to err instead of panic
        })
        .unwrap_or_default();

    // TODO errs
    output_data.get_mut(..output.len()).unwrap().copy_from_slice(&output);

    
    // TODO do things after writing into output
    todo!();
}
