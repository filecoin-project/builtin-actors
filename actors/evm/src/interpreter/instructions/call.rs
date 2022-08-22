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

    // if state.evm_revision >= Revision::Berlin
    //         && ResumeDataVariant::into_access_account_status(
    //             $co.yield_(InterruptDataVariant::AccessAccount(AccessAccount {
    //                 address: dst,
    //             }))
    //             .await,
    //         )
    //         .unwrap()
    //         .status
    //             == AccessStatus::Cold
    //     {
    //         $state.gas_left -= i64::from(ADDITIONAL_COLD_ACCOUNT_ACCESS_COST);
    //         if $state.gas_left < 0 {
    //             return Err(StatusCode::OutOfGas);
    //         }
    //     }

    // $state.gas_left -= i64::from(ADDITIONAL_COLD_ACCOUNT_ACCESS_COST);
    //         if $state.gas_left < 0 {
    //             return Err(StatusCode::OutOfGas);
    //         }

    //TODO Errs
    let input_region = get_memory_region(memory, input_offset, input_size).unwrap();
    let output_region = get_memory_region(memory, output_offset, output_size).unwrap();

    // let input_region = memory::verify_memory_region(state, input_offset, input_size)
    //     .map_err(|_| StatusCode::OutOfGas)?;
    // let output_region = memory::verify_memory_region(state, output_offset, output_size)
    //     .map_err(|_| StatusCode::OutOfGas)?;

    // let mut msg = Message {
    //     kind: kind,
    //     is_static: is_static, // ?? || state.message.is_static,
    //     depth: state.message.depth + 1,
    //     recipient: if matches!(kind, CallKind::Call) { dst } else { state.message.recipient },
    //     sender: if matches!(kind, CallKind::DelegateCall) {
    //         state.message.sender
    //     } else {
    //         state.message.recipient
    //     },
    //     gas: i64::MAX,
    //     value: if matches!(kind, CallKind::DelegateCall) { state.message.value } else { value },
    //     input_data: input_region
    //         .map(|MemoryRegion { offset, size }| {
    //             state.memory[offset..offset + size.get()].to_vec().into()
    //         })
    //         .unwrap_or_default(),
    // };

    let output = {
        // drop input data so we can mutate with output
        let input_data = input_region
            .map(|MemoryRegion { offset, size }| &memory[offset..offset + size.get()])
            .unwrap_or_default();

        let output = if dst <= precompiles::MAX_PRECOMPILE {
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

    
    
    // i dont like writing out into a vec like this, weird
    // output_data.
    // TODO do things after message
    todo!();
}
