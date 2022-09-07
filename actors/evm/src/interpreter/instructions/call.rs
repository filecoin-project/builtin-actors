use {
    super::memory::get_memory_region,
    crate::interpreter::address::Address,
    crate::interpreter::instructions::memory::MemoryRegion,
    crate::interpreter::output::StatusCode,
    crate::interpreter::precompiles,
    crate::interpreter::stack::Stack,
    crate::interpreter::ExecutionState,
    crate::interpreter::System,
    crate::interpreter::U256,
    crate::RawBytes,
    crate::{InvokeParams, Method},
    fil_actors_runtime::runtime::builtins::Type as ActorType,
    fil_actors_runtime::runtime::Runtime,
    fvm_ipld_blockstore::Blockstore,
    fvm_shared::econ::TokenAmount,
};

/// The kind of call-like instruction.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CallKind {
    Call,
    DelegateCall,
    StaticCall,
    CallCode,
}

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

pub fn calldatacopy(state: &mut ExecutionState) -> Result<(), StatusCode> {
    let mem_index = state.stack.pop();
    let input_index = state.stack.pop();
    let size = state.stack.pop();

    let region = get_memory_region(&mut state.memory, mem_index, size)
        .map_err(|_| StatusCode::InvalidMemoryAccess)?;

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

pub fn codecopy(state: &mut ExecutionState, code: &[u8]) -> Result<(), StatusCode> {
    let mem_index = state.stack.pop();
    let input_index = state.stack.pop();
    let size = state.stack.pop();

    let region = get_memory_region(&mut state.memory, mem_index, size)
        .map_err(|_| StatusCode::InvalidMemoryAccess)?;

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

pub fn call<'r, BS: Blockstore, RT: Runtime<BS>>(
    state: &mut ExecutionState,
    platform: &'r System<'r, BS, RT>,
    kind: CallKind,
) -> Result<(), StatusCode> {
    let ExecutionState { stack, memory, .. } = state;
    let rt = &*platform.rt; // as immutable reference

    let (_gas, dst, value, input_offset, input_size, output_offset, output_size) = match kind {
        CallKind::Call | CallKind::CallCode => (
            stack.pop(),
            stack.pop(),
            stack.pop(),
            stack.pop(),
            stack.pop(),
            stack.pop(),
            stack.pop(),
        ),

        CallKind::DelegateCall | CallKind::StaticCall => (
            stack.pop(),
            stack.pop(),
            U256::from(0),
            stack.pop(),
            stack.pop(),
            stack.pop(),
            stack.pop(),
        ),
    };

    let input_region = get_memory_region(memory, input_offset, input_size)
        .map_err(|_| StatusCode::InvalidMemoryAccess)?;
    let output_region = get_memory_region(memory, output_offset, output_size)
        .map_err(|_| StatusCode::InvalidMemoryAccess)?;

    let output = {
        // ref to memory is dropped after calling so we can mutate it on output later
        let input_data = input_region
            .map(|MemoryRegion { offset, size }| &memory[offset..][..size.get()])
            .ok_or(StatusCode::InvalidMemoryAccess)?;

        if precompiles::Precompiles::<BS, RT>::is_precompile(&dst) {
            precompiles::Precompiles::call_precompile(rt, dst, input_data)
                .map_err(|_| StatusCode::PrecompileFailure)?
        } else {
            // CALL and its brethren can only invoke other EVM contracts; see the (magic)
            // CALLMETHOD/METHODNUM opcodes for calling fil actors with native call
            // conventions.
            let dst_addr = Address::from(dst)
                .as_id_address()
                .ok_or(StatusCode::BadAddress("not an actor id address".to_string()))?;

            let dst_code_cid = rt
                .get_actor_code_cid(
                    &rt.resolve_address(&dst_addr).ok_or(StatusCode::BadAddress(
                        "cannot resolve address".to_string(),
                    ))?,
                )
                .ok_or(StatusCode::BadAddress("unknow actor".to_string()))?;
            let evm_code_cid = rt.get_code_cid_for_type(ActorType::EVM);
            if dst_code_cid != evm_code_cid {
                return Err(StatusCode::BadAddress(
                    "cannot call non EVM actor".to_string(),
                ));
            }

            match kind {
                CallKind::Call => {
                    let params = InvokeParams { input_data: RawBytes::from(input_data.to_vec()) };
                    let result = rt.send(
                        &dst_addr,
                        Method::InvokeContract as u64,
                        RawBytes::serialize(params).map_err(|_| {
                            StatusCode::InternalError(
                                "failed to marshall invocation data".to_string(),
                            )
                        })?,
                        TokenAmount::from(&value),
                    );
                    result.map_err(|e| StatusCode::from(e))?.to_vec()
                }
                CallKind::DelegateCall => {
                    todo!()
                }
                CallKind::StaticCall => {
                    todo!()
                }
                CallKind::CallCode => {
                    todo!()
                }
            }
        }
    };

    let output_data = output_region
        .map(|MemoryRegion { offset, size }| {
            &mut memory[offset..][..size.get()] // would like to use get for this to err instead of panic
        })
        .ok_or(StatusCode::InvalidMemoryAccess)?;

    output_data
        .get_mut(..output.len())
        .ok_or(StatusCode::InvalidMemoryAccess)?
        .copy_from_slice(&output);

    stack.push(U256::from(1));
    Ok(())
}
