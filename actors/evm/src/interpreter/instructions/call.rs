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
    crate::{Method, EVM_CONTRACT_REVERTED},
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

    // NOTE gas is currently ignored as FVM's send doesn't allow the caller to specify a gas
    //      limit (external invocation gas limit applies). This may changed in the future.
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

    let result = {
        // ref to memory is dropped after calling so we can mutate it on output later
        let input_data = if let Some(MemoryRegion { offset, size }) = input_region {
            &memory[offset..][..size.get()]
        } else {
            &[]
        };

        if precompiles::Precompiles::<BS, RT>::is_precompile(&dst) {
            let result = precompiles::Precompiles::call_precompile(rt, dst, input_data)
                .map_err(|_| StatusCode::PrecompileFailure)?;
            Ok(RawBytes::from(result))
        } else {
            let dst_addr = Address::try_from(dst)?
                .as_id_address()
                .ok_or_else(|| StatusCode::BadAddress("not an actor id address".to_string()))?;

            match kind {
                CallKind::Call => rt.send(
                    &dst_addr,
                    Method::InvokeContract as u64,
                    RawBytes::from(input_data.to_vec()),
                    TokenAmount::from(&value),
                ),
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

    if let Err(ae) = result {
        return if ae.exit_code() == EVM_CONTRACT_REVERTED {
            // reverted -- we don't have return data yet
            // push failure
            stack.push(U256::zero());
            Ok(())
        } else {
            Err(StatusCode::from(ae))
        };
    }

    let mut result = result.unwrap().to_vec();

    // save return_data
    state.return_data = result.clone().into();

    // copy return data to output region if it is non-zero
    // TODO this limits addressable output to 2G (31 bits full),
    //      but it is still probably too much and we should consistently limit further.
    //      See also https://github.com/filecoin-project/ref-fvm/issues/851
    if output_size.bits() >= 32 {
        return Err(StatusCode::InvalidMemoryAccess);
    }
    let output_usize = output_size.as_usize();

    if output_usize > 0 {
        let output_region = get_memory_region(memory, output_offset, output_size)
            .map_err(|_| StatusCode::InvalidMemoryAccess)?;
        let output_data = output_region
            .map(|MemoryRegion { offset, size }| &mut memory[offset..][..size.get()])
            .ok_or(StatusCode::InvalidMemoryAccess)?;

        // truncate if needed
        let mut result_usize = result.len();
        if result_usize > output_usize {
            result_usize = output_usize;
            result.truncate(output_usize);
        }

        output_data
            .get_mut(..result_usize)
            .ok_or(StatusCode::InvalidMemoryAccess)?
            .copy_from_slice(&result);
    }

    stack.push(U256::from(1));
    Ok(())
}

pub fn callactor<'r, BS: Blockstore, RT: Runtime<BS>>(
    state: &mut ExecutionState,
    platform: &'r System<'r, BS, RT>,
) -> Result<(), StatusCode> {
    let ExecutionState { stack, memory, .. } = state;
    let rt = &*platform.rt; // as immutable reference

    // stack: GAS DEST VALUE METHODNUM INPUT-OFFSET INPUT-SIZE
    // NOTE: we don't need output-offset/output-size (which the CALL instructions have)
    //       becase these are kinda useless; we can just use RETURNDATA anyway.
    // NOTE: gas is currently ignored
    let _gas = stack.pop();
    let dst = stack.pop();
    let value = stack.pop();
    let method = stack.pop();
    let input_offset = stack.pop();
    let input_size = stack.pop();

    let input_region = get_memory_region(memory, input_offset, input_size)
        .map_err(|_| StatusCode::InvalidMemoryAccess)?;

    let result = {
        let dst_addr = Address::try_from(dst)?
            .as_id_address()
            .ok_or_else(|| StatusCode::BadAddress(format!("not an actor id address: {}", dst)))?;

        if method.bits() > 64 {
            return Err(StatusCode::ArgumentOutOfRange(format!("bad method number: {}", method)));
        }
        let methodnum = method.as_u64();

        let input_data = if let Some(MemoryRegion { offset, size }) = input_region {
            &memory[offset..][..size.get()]
        } else {
            &[]
        }
        .to_vec();
        rt.send(&dst_addr, methodnum, RawBytes::from(input_data), TokenAmount::from(&value))
    };

    if let Err(ae) = result {
        return if ae.exit_code() == EVM_CONTRACT_REVERTED {
            // reverted -- we don't have return data yet
            // push failure
            stack.push(U256::zero());
            Ok(())
        } else {
            Err(StatusCode::from(ae))
        };
    }

    // save return_data
    state.return_data = result.unwrap().to_vec().into();
    // push success
    stack.push(U256::from(1));
    Ok(())
}

pub fn methodnum(state: &mut ExecutionState) {
    state.stack.push(U256::from(state.method));
}
