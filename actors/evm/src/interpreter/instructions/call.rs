#![allow(clippy::too_many_arguments)]

use fil_actors_evm_shared::{address::EthAddress, uints::U256};
use fvm_ipld_encoding::ipld_block::IpldBlock;
use fvm_ipld_encoding::BytesDe;
use fvm_shared::{address::Address, sys::SendFlags, MethodNum, IPLD_RAW};

use crate::interpreter::{
    precompiles::{is_reserved_precompile_address, PrecompileContext},
    CallKind,
};

use super::ext::{get_contract_type, get_evm_bytecode_cid, ContractType};

use {
    super::memory::{copy_to_memory, get_memory_region},
    crate::interpreter::instructions::memory::MemoryRegion,
    crate::interpreter::precompiles,
    crate::interpreter::ExecutionState,
    crate::interpreter::System,
    crate::{DelegateCallParams, Method},
    fil_actors_runtime::runtime::Runtime,
    fil_actors_runtime::ActorError,
    fvm_shared::econ::TokenAmount,
    fvm_shared::error::ErrorNumber,
};

/// The gas granted on bare "transfers".
const TRANSFER_GAS_LIMIT: U256 = U256::from_u64(10_000_000);

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
            let end = core::cmp::min(start.saturating_add(crate::EVM_WORD_SIZE), input_len);
            let mut data = [0; crate::EVM_WORD_SIZE];
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

    let (mut gas, dst, value, input_offset, input_size, output_offset, output_size) = params;

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
                    if (gas == 0 && value > 0) || (gas == 2300 && value == 0) {
                        // We provide enough gas for the transfer to succeed in all case.
                        gas = TRANSFER_GAS_LIMIT;
                    }
                    let gas_limit = Some(effective_gas_limit(system, gas));
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
                    // Error cases:
                    //
                    // 1. If the outer result fails, it means we failed to flush/restore state and
                    // there is a bug. We exit with an actor error and abort.
                    match system.send_raw(
                        &dst_addr,
                        Method::InvokeContract as MethodNum,
                        params,
                        value,
                        gas_limit,
                        send_flags,
                    )? {
                        Ok(resp) => {
                            if resp.exit_code.is_success() {
                                Ok(resp.return_data)
                            } else {
                                Err(resp.return_data)
                            }
                        }
                        Err(e) => match e {
                            // The target actor doesn't exist. To match EVM behavior, we walk away.
                            ErrorNumber::NotFound => Ok(None),
                            // If we hit this case, we must have tried to auto-deploy an actor
                            // while read-only. We've already checked that we aren't trying to
                            // transfer funds.
                            //
                            // To match EVM behavior, we treat this case as "success" and
                            // walk away.
                            ErrorNumber::ReadOnly
                                if system.readonly || kind == CallKind::StaticCall =>
                            {
                                Ok(None)
                            }
                            ErrorNumber::InsufficientFunds => Err(None),
                            ErrorNumber::LimitExceeded => Err(None),
                            // Nothing else is expected in this case. This likely indicates a bug, but
                            // it doesn't indicate that there's an issue with _this_ EVM actor, so we
                            // might as log and well continue.
                            e => {
                                log::error!("unexpected syscall error on CALL to {dst_addr}: {e}");
                                Err(None)
                            }
                        },
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
                            system
                                .send(
                                    &system.rt.message().receiver(),
                                    Method::InvokeContractDelegate as u64,
                                    IpldBlock::serialize_dag_cbor(&params)?,
                                    TokenAmount::from(&value),
                                    Some(effective_gas_limit(system, gas)),
                                    SendFlags::default(),
                                )
                                .map_err(|mut ae| ae.take_data())
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
                        log::info!("attempted to delegatecall a native actor at {dst:?}");
                        Err(None)
                    }
                    ContractType::Precompile => {
                        log::error!("reached a precompile address in DelegateCall when a precompile should've been caught earlier in the system");
                        Err(None)
                    }
                },
            };
            let (code, data) = match call_result {
                Ok(result) => (1, result),
                Err(result) => (0, result),
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

#[cfg(test)]
mod tests {
    use crate::evm_unit_test;
    use crate::interpreter::U256;

    #[test]
    fn test_calldataload() {
        evm_unit_test! {
            (rt, m) {
                CALLDATALOAD;
            }
            m.state.input_data = vec![0x00, 0x01, 0x02].into();
            m.state.stack.push(U256::from(1)).unwrap();
            let result = m.step();
            assert!(result.is_ok(), "execution step failed");
            assert_eq!(m.state.stack.len(), 1);
            assert_eq!(m.state.stack.pop().unwrap(), U256::from(0x0102) << 240);
        };
    }

    #[test]
    fn test_calldatasize() {
        evm_unit_test! {
            (rt, m) {
                CALLDATASIZE;
            }
            m.state.input_data = vec![0x00, 0x01, 0x02].into();
            let result = m.step();
            assert!(result.is_ok(), "execution step failed");
            assert_eq!(m.state.stack.len(), 1);
            assert_eq!(m.state.stack.pop().unwrap(), U256::from(3));
        };
    }

    #[test]
    fn test_calldatacopy() {
        evm_unit_test! {
            (rt, m) {
                CALLDATACOPY;
            }
            m.state.input_data = vec![0x00, 0x01, 0x02].into();
            m.state.stack.push(U256::from(2)).unwrap();  // length
            m.state.stack.push(U256::from(1)).unwrap();  // offset
            m.state.stack.push(U256::from(0)).unwrap();  // dest-offset
            let result = m.step();
            assert!(result.is_ok(), "execution step failed");
            assert_eq!(m.state.stack.len(), 0);
            let mut expected = [0u8; 32];
            expected[0] = 0x01;
            expected[1] = 0x02;
            assert_eq!(m.state.memory.as_ref(), &expected);
        };
    }
}
