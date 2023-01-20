#![allow(clippy::too_many_arguments)]

use fvm_ipld_encoding::ipld_block::IpldBlock;
use fvm_ipld_encoding::BytesDe;
use fvm_shared::{address::Address, sys::SendFlags, IPLD_RAW, METHOD_SEND};

use crate::interpreter::precompiles::{is_reserved_precompile_address, PrecompileContext};

use super::ext::{get_contract_type, get_evm_bytecode_cid, ContractType};

use {
    super::memory::{copy_to_memory, get_memory_region},
    crate::interpreter::address::EthAddress,
    crate::interpreter::instructions::memory::MemoryRegion,
    crate::interpreter::precompiles,
    crate::interpreter::ExecutionState,
    crate::interpreter::System,
    crate::interpreter::U256,
    crate::{DelegateCallParams, Method},
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
}

pub fn calldataload(
    state: &mut ExecutionState,
    _: &System<impl Runtime>,
    index: U256,
) -> Result<U256, ActorError> {
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
) -> Result<U256, ActorError> {
    Ok(u128::try_from(state.input_data.len()).unwrap().into())
}

#[inline]
pub fn calldatacopy(
    state: &mut ExecutionState,
    _: &System<impl Runtime>,
    mem_index: U256,
    input_index: U256,
    size: U256,
) -> Result<(), ActorError> {
    copy_to_memory(&mut state.memory, mem_index, size, input_index, &state.input_data, true)
}

#[inline]
pub fn codesize(
    _state: &mut ExecutionState,
    _: &System<impl Runtime>,
    code: &[u8],
) -> Result<U256, ActorError> {
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
) -> Result<(), ActorError> {
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
) -> Result<U256, ActorError> {
    call_generic(
        state,
        system,
        CallKind::Call,
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
) -> Result<U256, ActorError> {
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
) -> Result<U256, ActorError> {
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
) -> Result<U256, ActorError> {
    let ExecutionState { stack: _, memory, .. } = state;

    let (gas, dst, value, input_offset, input_size, output_offset, output_size) = params;

    if system.readonly && value > U256::zero() {
        // non-zero sends are side-effects and hence a static mode violation
        return Err(ActorError::read_only("cannot transfer value when read-only".into()));
    }

    let input_region = get_memory_region(memory, input_offset, input_size)?;

    let (call_result, return_data) = {
        // ref to memory is dropped after calling so we can mutate it on output later
        let input_data = if let Some(MemoryRegion { offset, size }) = input_region {
            &memory[offset..][..size.get()]
        } else {
            &[]
        };

        let dst: EthAddress = dst.into();
        if is_reserved_precompile_address(&dst) {
            let context = PrecompileContext {
                call_type: kind,
                gas_limit: effective_gas_limit(system, gas),
                value,
            };

            if log::log_enabled!(log::Level::Info) {
                // log input to the precompile, but make sure we dont log _too_ much.
                let mut input_hex = hex::encode(input_data);
                input_hex.truncate(1024);
                if input_data.len() > 512 {
                    input_hex.push_str("[..]")
                }
                log::info!(target: "evm", "Call Precompile:\n\taddress: {:x?}\n\tcontext: {:?}\n\tinput: {}", dst, context, input_hex);
            }

            match precompiles::Precompiles::call_precompile(system, &dst, input_data, context) {
                Ok(return_data) => (1, return_data),
                Err(err) => {
                    log::warn!(target: "evm", "Precompile failed: error {:?}", err);
                    // precompile failed, exit with reverted and no output
                    (0, vec![])
                }
            }
        } else {
            let call_result = match kind {
                CallKind::Call | CallKind::StaticCall => {
                    let dst_addr: Address = dst.into();

                    // Special casing for account/placeholder/non-existent actors: we just do a SEND (method 0)
                    // which allows us to transfer funds (and create placeholders)
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
                        Ok(None)
                    } else {
                        let (method, gas_limit) = if !actor_exists
                            || matches!(target_actor_type, Some(Type::Placeholder | Type::Account | Type::EthAccount))
                            // See https://github.com/filecoin-project/ref-fvm/issues/980 for this
                            // hocus pocus
                            || (input_data.is_empty() && ((gas == 0 && value > 0) || (gas == 2300 && value == 0)))
                        {
                            // We switch to a bare send when:
                            //
                            // 1. The target is a placeholder/account or doesn't exist. Otherwise,
                            // sending funds to an account/placeholder would fail when we try to call
                            // InvokeContract.
                            // 2. The gas wouldn't let code execute anyways. This lets us support
                            // solidity's "transfer" method.
                            //
                            // At the same time, we ignore the supplied gas value and set it to
                            // infinity as user code won't execute anyways. The only code that might
                            // run is related to account creation, which doesn't count against this
                            // gas limit in the EVM anyways.
                            (METHOD_SEND, None)
                        } else {
                            // Otherwise, invoke normally.
                            (Method::InvokeContract as u64, Some(effective_gas_limit(system, gas)))
                        };
                        let params = if input_data.is_empty() {
                            None
                        } else {
                            Some(IpldBlock { codec: IPLD_RAW, data: input_data.into() })
                        };
                        let value = TokenAmount::from(&value);
                        let send_flags = if kind == CallKind::StaticCall {
                            SendFlags::READ_ONLY
                        } else {
                            SendFlags::default()
                        };
                        system.send(&dst_addr, method, params, value, gas_limit, send_flags)
                    }
                }
                CallKind::DelegateCall => match get_contract_type(system.rt, &dst) {
                    ContractType::EVM(dst_addr) => {
                        // If we're calling an actual EVM actor, get its code.
                        if let Some(code) = get_evm_bytecode_cid(system, &dst_addr)? {
                            // and then invoke self with delegate; readonly context is sticky
                            let params = DelegateCallParams {
                                code,
                                input: input_data.into(),
                                caller: state.caller,
                                value: state.value_received.clone(),
                            };
                            system.send(
                                &system.rt.message().receiver(),
                                Method::InvokeContractDelegate as u64,
                                IpldBlock::serialize_cbor(&params)?,
                                TokenAmount::from(&value),
                                Some(effective_gas_limit(system, gas)),
                                SendFlags::default(),
                            )
                        } else {
                            // If it doesn't have code, short-circuit and return immediately.
                            Ok(None)
                        }
                    }
                    // If we're calling an account or a non-existent actor, return nothing because
                    // this is how the EVM behaves.
                    ContractType::Account | ContractType::NotFound => Ok(None),
                    // If we're calling a "native" actor, always revert.
                    ContractType::Native(_) => {
                        Err(ActorError::forbidden("cannot delegate-call to native actors".into()))
                    }
                    ContractType::Precompile => Err(ActorError::assertion_failed(
                        "Reached a precompile address in DelegateCall when a precompile should've been caught earlier in the system"
                            .to_string(),
                    )),
                },
            };
            let (code, data) = match call_result {
                Ok(result) => (1, result),
                Err(mut ae) => (0, ae.take_data()),
            };

            (
                code,
                match data {
                    // Support the "empty" result. We often use this to mean "returned nothing" and
                    // it's important to support, e.g., sending to accounts.
                    None => Vec::new(),
                    Some(r) =>
                    // NOTE: If the user returns an invalid thing, we just the returned bytes as-is.
                    // We can't lie to the contract and say that the callee reverted, and we don't want
                    // to "abort".
                    {
                        r.deserialize().map(|BytesDe(d)| d).unwrap_or_else(|_| r.data)
                    }
                },
            )
        }
    };

    state.return_data = return_data.into();

    // copy return data to output region if it is non-zero
    copy_to_memory(memory, output_offset, output_size, U256::zero(), &state.return_data, false)?;

    Ok(U256::from(call_result))
}

fn effective_gas_limit<RT: Runtime>(system: &System<RT>, gas: U256) -> u64 {
    let gas_rsvp = (63 * system.rt.gas_available()) / 64;
    let gas = gas.to_u64_saturating();
    std::cmp::min(gas, gas_rsvp)
}
