#![allow(clippy::too_many_arguments)]

use fvm_ipld_encoding::{BytesDe, BytesSer};
use fvm_shared::{address::Address, sys::SendFlags, METHOD_SEND};

use crate::interpreter::precompiles::PrecompileContext;

use super::ext::{get_cid_type, get_evm_bytecode_cid, ContractType};

use {
    super::memory::{copy_to_memory, get_memory_region},
    crate::interpreter::address::EthAddress,
    crate::interpreter::instructions::memory::MemoryRegion,
    crate::interpreter::output::StatusCode,
    crate::interpreter::precompiles,
    crate::interpreter::ExecutionState,
    crate::interpreter::System,
    crate::interpreter::U256,
    crate::RawBytes,
    crate::{DelegateCallParams, Method, EVM_CONTRACT_EXECUTION_ERROR},
    fil_actors_runtime::runtime::builtins::Type,
    fil_actors_runtime::runtime::Runtime,
    fil_actors_runtime::ActorError,
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

pub fn calldataload(
    state: &mut ExecutionState,
    _: &System<impl Runtime>,
    index: U256,
) -> Result<U256, StatusCode> {
    let input_len = state.input_data.len();
    Ok(index
        .try_into()
        .ok()
        .filter(|&start| start < input_len)
        .map(|start: usize| {
            let end = core::cmp::min(start.saturating_add(32usize), input_len);
            let mut data = [0; 32];
            data[..end - start].copy_from_slice(&state.input_data[start..end]);
            U256::from_big_endian(&data)
        })
        .unwrap_or_default())
}

#[inline]
pub fn calldatasize(
    state: &mut ExecutionState,
    _: &System<impl Runtime>,
) -> Result<U256, StatusCode> {
    Ok(u128::try_from(state.input_data.len()).unwrap().into())
}

#[inline]
pub fn calldatacopy(
    state: &mut ExecutionState,
    _: &System<impl Runtime>,
    mem_index: U256,
    input_index: U256,
    size: U256,
) -> Result<(), StatusCode> {
    copy_to_memory(&mut state.memory, mem_index, size, input_index, &state.input_data, true)
}

#[inline]
pub fn codesize(
    _state: &mut ExecutionState,
    _: &System<impl Runtime>,
    code: &[u8],
) -> Result<U256, StatusCode> {
    Ok(U256::from(code.len()))
}

#[inline]
pub fn codecopy(
    state: &mut ExecutionState,
    _: &System<impl Runtime>,
    code: &[u8],
    mem_index: U256,
    input_index: U256,
    size: U256,
) -> Result<(), StatusCode> {
    copy_to_memory(&mut state.memory, mem_index, size, input_index, code, true)
}

#[inline]
pub fn call_call<RT: Runtime>(
    state: &mut ExecutionState,
    system: &mut System<RT>,
    gas: U256,
    dst: U256,
    value: U256,
    input_offset: U256,
    input_size: U256,
    output_offset: U256,
    output_size: U256,
) -> Result<U256, StatusCode> {
    call_generic(
        state,
        system,
        CallKind::Call,
        (gas, dst, value, input_offset, input_size, output_offset, output_size),
    )
}

#[inline]
pub fn call_callcode<RT: Runtime>(
    state: &mut ExecutionState,
    system: &mut System<RT>,
    gas: U256,
    dst: U256,
    value: U256,
    input_offset: U256,
    input_size: U256,
    output_offset: U256,
    output_size: U256,
) -> Result<U256, StatusCode> {
    call_generic(
        state,
        system,
        CallKind::CallCode,
        (gas, dst, value, input_offset, input_size, output_offset, output_size),
    )
}

#[inline]
pub fn call_delegatecall<RT: Runtime>(
    state: &mut ExecutionState,
    system: &mut System<RT>,
    gas: U256,
    dst: U256,
    input_offset: U256,
    input_size: U256,
    output_offset: U256,
    output_size: U256,
) -> Result<U256, StatusCode> {
    call_generic(
        state,
        system,
        CallKind::DelegateCall,
        (gas, dst, U256::zero(), input_offset, input_size, output_offset, output_size),
    )
}

#[inline]
pub fn call_staticcall<RT: Runtime>(
    state: &mut ExecutionState,
    system: &mut System<RT>,
    gas: U256,
    dst: U256,
    input_offset: U256,
    input_size: U256,
    output_offset: U256,
    output_size: U256,
) -> Result<U256, StatusCode> {
    call_generic(
        state,
        system,
        CallKind::StaticCall,
        (gas, dst, U256::zero(), input_offset, input_size, output_offset, output_size),
    )
}

pub fn call_generic<RT: Runtime>(
    state: &mut ExecutionState,
    system: &mut System<RT>,
    kind: CallKind,
    params: (U256, U256, U256, U256, U256, U256, U256),
) -> Result<U256, StatusCode> {
    let ExecutionState { stack: _, memory, .. } = state;

    let (gas, dst, value, input_offset, input_size, output_offset, output_size) = params;

    if system.readonly && value > U256::zero() {
        // non-zero sends are side-effects and hence a static mode violation
        return Err(StatusCode::StaticModeViolation);
    }

    let input_region = get_memory_region(memory, input_offset, input_size)
        .map_err(|_| StatusCode::InvalidMemoryAccess)?;

    let (call_result, return_data) = {
        // ref to memory is dropped after calling so we can mutate it on output later
        let input_data = if let Some(MemoryRegion { offset, size }) = input_region {
            &memory[offset..][..size.get()]
        } else {
            &[]
        };

        if precompiles::Precompiles::<RT>::is_precompile(&dst) {
            let context = PrecompileContext {
                call_type: kind,
                is_readonly: system.readonly,
                gas_limit: effective_gas_limit(system, gas),
            };

            match precompiles::Precompiles::call_precompile(system, dst, input_data, context)
                .map_err(StatusCode::from)
            {
                Ok(return_data) => (1, return_data),
                Err(status) => {
                    let msg = format!("{}", status);
                    (0, msg.as_bytes().to_vec())
                }
            }
        } else {
            let call_result = match kind {
                CallKind::Call | CallKind::StaticCall => {
                    let dst_addr: EthAddress = dst.into();
                    let dst_addr: Address = dst_addr.try_into().expect("address is a precompile");

                    // Special casing for account/embryo/non-existent actors: we just do a SEND (method 0)
                    // which allows us to transfer funds (and create embryos)
                    let target_actor_code = system
                        .rt
                        .resolve_address(&dst_addr)
                        .and_then(|actor_id| system.rt.get_actor_code_cid(&actor_id));
                    let target_actor_type = target_actor_code
                        .as_ref()
                        .and_then(|cid| system.rt.resolve_builtin_actor_type(cid));
                    let actor_exists = target_actor_code.is_some();

                    if !actor_exists && value.is_zero() {
                        // If the actor doesn't exist and we're not sending value, return with
                        // "success". The EVM only auto-creates actors when sending value.
                        //
                        // NOTE: this will also apply if we're in read-only mode, because we can't
                        // send value in read-only mode anyways.
                        Ok(RawBytes::default())
                    } else {
                        let method = if !actor_exists
                            || matches!(target_actor_type, Some(Type::Embryo | Type::Account))
                        // See https://github.com/filecoin-project/ref-fvm/issues/980 for this
                        // magic value
                            || (!value.is_zero() && !gas.is_zero() && gas <= U256::from(21_000))
                        {
                            // If the target actor doesn't exist or is an account or an embryo,
                            // switch to a basic "send" so the call will still work even if the
                            // target actor would reject a normal ethereum call.
                            METHOD_SEND
                        } else {
                            // Otherwise, invoke normally.
                            Method::InvokeContract as u64
                        };
                        // TODO: support IPLD codecs #758
                        let params = RawBytes::serialize(BytesSer(input_data))?;
                        let value = TokenAmount::from(&value);
                        let gas_limit = effective_gas_limit(system, gas);
                        let read_only = if kind == CallKind::StaticCall {
                            SendFlags::READ_ONLY
                        } else {
                            SendFlags::default()
                        };
                        system.send_generalized(
                            &dst_addr, method, params, value, gas_limit, read_only,
                        )
                    }
                }
                CallKind::DelegateCall => match get_cid_type(system.rt, dst) {
                    ContractType::EVM(dst_addr) => {
                        // If we're calling an actual EVM actor, get it's code.
                        let code = get_evm_bytecode_cid(system.rt, &dst_addr)?;

                        // and then invoke self with delegate; readonly context is sticky
                        let params = DelegateCallParams { code, input: input_data.into() };
                        system.send_generalized(
                            &system.rt.message().receiver(),
                            Method::InvokeContractDelegate as u64,
                            RawBytes::serialize(&params)?,
                            TokenAmount::from(&value),
                            effective_gas_limit(system, gas),
                            if system.readonly {
                                SendFlags::READ_ONLY
                            } else {
                                SendFlags::default()
                            },
                        )
                    }
                    // If we're calling an account or a non-existent actor, return nothing because
                    // this is how the EVM behaves.
                    ContractType::Account | ContractType::NotFound => Ok(RawBytes::default()),
                    // If we're calling a "native" actor, always revert.
                    ContractType::Native(_) => {
                        Err(ActorError::forbidden("cannot delegate-call to native actors".into()))
                    }
                },
                CallKind::CallCode => Err(ActorError::unchecked(
                    EVM_CONTRACT_EXECUTION_ERROR,
                    "unsupported opcode".to_string(),
                )),
            };
            match call_result {
                Ok(result) => {
                    // Support the "empty" result. We often use this to mean "returned nothing" and
                    // it's important to support, e.g., sending to accounts.
                    if result.is_empty() {
                        (1, Vec::new())
                    } else {
                        // TODO: support IPLD codecs #758
                        let BytesDe(result) = result.deserialize()?;
                        (1, result)
                    }
                }
                Err(mut ae) => (0, ae.take_data().into()),
            }
        }
    };

    state.return_data = return_data.into();

    // copy return data to output region if it is non-zero
    copy_to_memory(memory, output_offset, output_size, U256::zero(), &state.return_data, false)?;

    Ok(U256::from(call_result))
}

fn effective_gas_limit<RT: Runtime>(system: &System<RT>, gas: U256) -> Option<u64> {
    let gas_rsvp = (63 * system.rt.gas_available()) / 64;
    Some(if gas.is_zero() {
        gas_rsvp
    } else {
        let gas = gas.to_u64_saturating();
        std::cmp::min(gas, gas_rsvp)
    })
}
