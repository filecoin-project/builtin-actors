#![allow(clippy::too_many_arguments)]

use fvm_ipld_encoding::ipld_block::IpldBlock;
use fvm_ipld_encoding::BytesDe;
use fvm_shared::{address::Address, sys::SendFlags, MethodNum, IPLD_RAW, METHOD_SEND};
use num_traits::Zero;

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
    fil_actors_runtime::runtime::Runtime,
    fil_actors_runtime::ActorError,
    fvm_shared::econ::TokenAmount,
    fvm_shared::error::ErrorNumber,
};

/// The kind of call-like instruction.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CallKind {
    Call,
    DelegateCall,
    StaticCall,
}

/// For "transfers", we give 2M gas.
///
/// The actual call is expected to coss less than 1M gas, so we give it two to cover other
/// overheads. We can tune this number later (but it depends on integration testing with final gas
/// values).
const TRANSFER_GAS_LIMIT: u64 = 2_000_000;

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

    let (success, return_data) = {
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
                gas_limit: effective_gas_limit(system, gas.to_u64_saturating()),
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
                Ok(return_data) => (true, return_data),
                Err(err) => {
                    log::warn!(target: "evm", "Precompile failed: error {:?}", err);
                    // precompile failed, exit with reverted and no output
                    (false, vec![])
                }
            }
        } else {
            let (success, data) = match kind {
                CallKind::Call | CallKind::StaticCall => call_contract(
                    system,
                    &dst,
                    value.into(),
                    input_data,
                    gas.to_u64_saturating(),
                    kind == CallKind::StaticCall,
                )?,
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
                            match system.send(
                                &system.rt.message().receiver(),
                                Method::InvokeContractDelegate as u64,
                                IpldBlock::serialize_dag_cbor(&params)?,
                                TokenAmount::from(&value),
                                Some(effective_gas_limit(system, gas.to_u64_saturating())),
                                SendFlags::default(),
                            ) {
                                Ok(ret) => (true, ret),
                                Err(mut ae) => (false, ae.take_data()),
                            }
                        } else {
                            // If it doesn't have code, short-circuit and return immediately.
                            (true, None)
                        }
                    }
                    // If we're calling an account or a non-existent actor, return nothing because
                    // this is how the EVM behaves.
                    ContractType::Account | ContractType::NotFound => (true, None),
                    // If we're calling a "native" actor, always revert.
                    ContractType::Native(_) => {
                        log::info!("attempted to delegatecall a native actor at {dst:?}");
                        (false, None)
                    }
                    ContractType::Precompile => {
                        log::error!("reached a precompile address in DelegateCall when a precompile should've been caught earlier in the system");
                        (false, None)
                    }
                },
            };

            (
                success,
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

    Ok(U256::from(success as u8))
}

/// Actually call another contract.
fn call_contract(
    system: &mut System<impl Runtime>,
    to: &EthAddress,
    value: TokenAmount,
    input_data: &[u8],
    mut gas: u64,
    call_readonly: bool,
) -> Result<(bool, Option<IpldBlock>), ActorError> {
    let dst: Address = to.into();
    let send_flags = if call_readonly { SendFlags::READ_ONLY } else { SendFlags::default() };

    let params = if input_data.is_empty() {
        None
    } else {
        Some(IpldBlock { codec: IPLD_RAW, data: input_data.into() })
    };

    // Solidity "transfer" hack. If this looks like a solidity transfer, we fixup the gas values so
    // they work.
    if (gas == 0 && value.is_positive()) || (gas == 2300 && value.is_zero()) {
        // Ideally we'd just pick a reasonable gas value, but we can't quite do that because the
        // required gas varies based on whether or not the target actor exists, its address is warm,
        // its state is warm, etc... We'd have to reserve at least 6-7M gas.
        //
        // So we take some steps to "pre-warm" some things before we do the actual transfer.

        // First, we resolve the address (not cheap).
        if let Some(id) = system.rt.resolve_address(&dst) {
            // If that works, we look up the target actor's code CID. That:
            //
            // 1. Lets us check if it exists.
            // 2. Warms it.
            if system.rt.get_actor_code_cid(&id).is_none() {
                // If it doesn't exist but we successfully resolved the address, we might as well
                // fail now. Either:
                //
                // 1. The address is an ID address (i.e., an embedded ID address).
                // 2. The target actor existed at one point but no longer exists. Any sends would
                // fail in that case as well.
                return Ok((false, None));
            }
            gas = TRANSFER_GAS_LIMIT;
        } else if call_readonly || system.readonly {
            // If we're making a staticcall and the target actor doesn't exist, we don't want to
            // auto-create the target actor. Just return success (this is what the EVM would do).
            debug_assert!(
                value.is_zero(),
                "this method cannot be called with value and when read-only is set"
            );
            return Ok((true, None));
        } else {
            // Finally, we get here if:
            //
            // 1. The target actor doesn't exist.
            // 2. We're not performing a static call.
            // 3. Sending _might_ create the target actor (i.e., we don't have an ID address).
            //
            // So we just do a bare send and return.
            return match system.send(&dst, METHOD_SEND, None, TokenAmount::zero(), None, send_flags)
            {
                Ok(res) => Ok((true, res)),
                Err(mut ae) => Ok((false, ae.take_data())),
            };
        }
    }

    // Now apply the 63/64 rule.
    gas = effective_gas_limit(system, gas);

    // Error cases:
    //
    // 1. If the outer result fails, it means we failed to flush/restore state and
    // there is a bug. We exit with an actor error and abort.
    // 2. If the syscall fails, we have to carefully consider the error cases.
    match system.send_raw(
        &dst,
        Method::InvokeContract as MethodNum,
        params,
        value,
        Some(gas),
        send_flags,
    )? {
        Ok(resp) => Ok((resp.exit_code.is_success(), resp.return_data)),
        Err(e) => Ok((
            match e {
                // The target actor doesn't exist. To match EVM behavior, we call this success.
                ErrorNumber::NotFound => true,
                // If we hit this case, we must have tried to auto-deploy an actor
                // while read-only. We've already checked that we aren't trying to
                // transfer funds.
                //
                // To match EVM behavior, we treat this case as "success" and
                // walk away.
                ErrorNumber::ReadOnly if system.readonly || call_readonly => true,
                ErrorNumber::InsufficientFunds => false,
                ErrorNumber::LimitExceeded => false,
                // Nothing else is expected in this case. This likely indicates a bug, but
                // it doesn't indicate that there's an issue with _this_ EVM actor, so we
                // might as log and well continue.
                e => {
                    log::error!("unexpected syscall error on CALL to {dst}: {e}");
                    false
                }
            },
            None,
        )),
    }
}

fn effective_gas_limit<RT: Runtime>(system: &System<RT>, gas: u64) -> u64 {
    let gas_rsvp = (63 * system.rt.gas_available()) / 64;
    std::cmp::min(gas, gas_rsvp)
}
