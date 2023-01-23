use crate::EVM_MAX_RESERVED_METHOD;
use fil_actors_runtime::runtime::Runtime;
use fvm_ipld_encoding::ipld_block::IpldBlock;
use fvm_shared::{address::Address, econ::TokenAmount, sys::SendFlags, METHOD_SEND};

use crate::interpreter::{instructions::call::CallKind, System, U256};

use super::{PrecompileContext, PrecompileError, PrecompileResult};
use crate::reader::ValueReader;

/// Read BE encoded low u64 ID address from a u256 word
/// Looks up and returns the encoded f4 addresses of an ID address. Empty array if not found or `InvalidInput` input was larger 2^64.
pub(super) fn lookup_delegated_address<RT: Runtime>(
    system: &mut System<RT>,
    input: &[u8],
    _: PrecompileContext,
) -> PrecompileResult {
    let mut id_bytes = ValueReader::new(input);
    let id = id_bytes.read_value::<u64>()?;

    let address = system.rt.lookup_delegated_address(id);
    let ab = match address {
        Some(a) => a.to_bytes(),
        None => Vec::new(),
    };
    Ok(ab)
}

/// Reads a FIL (i.e. f0xxx, f4xfxxx) encoded address
/// Resolves a FIL encoded address into an ID address
/// Returns BE encoded u256 (return will always be under 2^64).
/// Empty array if nothing found or `InvalidInput` if length was larger 2^32 or Address parsing failed.
pub(super) fn resolve_address<RT: Runtime>(
    system: &mut System<RT>,
    input: &[u8],
    _: PrecompileContext,
) -> PrecompileResult {
    let addr = match Address::from_bytes(input) {
        Ok(o) => o,
        Err(e) => {
            log::debug!(target: "evm", "Address parsing failed: {e}");
            return Err(PrecompileError::InvalidInput);
        }
    };
    Ok(system
        .rt
        .resolve_address(&addr)
        .map(|a| {
            log::debug!(target: "evm", "{addr} resolved to {a}");
            U256::from(a).to_bytes().to_vec()
        })
        .unwrap_or_default())
}

/// Calls an actor by address.
///
/// Parameters are encoded according to the solidity ABI, with no function selector:
///
/// ```text
/// u64   method
/// u256  value
/// u64   flags (1 for read-only, 0 otherwise)
/// u64   codec (0x71 for "dag-cbor", or `0` for "nothing")
/// bytes params (must be empty if the codec is 0x0)
/// bytes address
/// ```
///
/// Returns (also solidity ABI encoded):
///
/// ```text
/// i256  exit_code
/// u64   codec
/// bytes return_value
/// ```
///
/// for exit_code:
/// - negative values are system errors
/// - positive are user errors (from the called actor)
/// - 0 is success
pub(super) fn call_actor<RT: Runtime>(
    system: &mut System<RT>,
    input: &[u8],
    ctx: PrecompileContext,
) -> PrecompileResult {
    call_actor_shared(system, input, ctx, false)
}

/// Calls an actor by the actor's actor ID.
///
/// Parameters are encoded according to the solidity ABI, with no function selector:
///
/// ```text
/// u64   method
/// u256  value
/// u64   flags (1 for read-only, 0 otherwise)
/// u64   codec (0x71 for "dag-cbor", or `0` for "nothing")
/// bytes params (must be empty if the codec is 0x0)
/// u64   actor_id
/// ```
///
/// Returns (also solidity ABI encoded):
///
/// ```text
/// i256  exit_code
/// u64   codec
/// bytes return_value
/// ```
///
/// for exit_code:
/// - negative values are system errors
/// - positive are user errors (from the called actor)
/// - 0 is success
pub(super) fn call_actor_id<RT: Runtime>(
    system: &mut System<RT>,
    input: &[u8],
    ctx: PrecompileContext,
) -> PrecompileResult {
    call_actor_shared(system, input, ctx, true)
}

pub(super) fn call_actor_shared<RT: Runtime>(
    system: &mut System<RT>,
    input: &[u8],
    ctx: PrecompileContext,
    by_id: bool,
) -> PrecompileResult {
    // ----- Input Parameters -------

    if ctx.call_type != CallKind::DelegateCall {
        return Err(PrecompileError::CallForbidden);
    }

    let mut input_params = ValueReader::new(input);

    let method: u64 = input_params.read_value()?;

    let value: U256 = input_params.read_value()?;

    let flags: u64 = input_params.read_value()?;
    let flags = SendFlags::from_bits(flags).ok_or(PrecompileError::InvalidInput)?;

    let codec: u64 = input_params.read_value()?;

    let params_off: u32 = input_params.read_value()?;
    let id_or_addr_off: u64 = input_params.read_value()?;

    input_params.seek(params_off.try_into()?);
    let params_len: u32 = input_params.read_value()?;
    let params = input_params.read_padded(params_len.try_into()?);

    let address = if by_id {
        Address::new_id(id_or_addr_off)
    } else {
        input_params.seek(id_or_addr_off.try_into()?);
        let addr_len: u32 = input_params.read_value()?;
        let addr_bytes = input_params
            .read_padded(addr_len.try_into().map_err(|_| PrecompileError::InvalidInput)?);
        Address::from_bytes(&addr_bytes).map_err(|_| PrecompileError::InvalidInput)?
    };

    if method <= EVM_MAX_RESERVED_METHOD && method != METHOD_SEND {
        return Err(PrecompileError::InvalidInput);
    }

    // ------ Begin Call -------

    let result = {
        // TODO only CBOR or "nothing" for now
        let params = match codec {
            fvm_ipld_encoding::CBOR => Some(IpldBlock { codec, data: params.into() }),
            #[cfg(feature = "hyperspace")]
            fvm_ipld_encoding::DAG_CBOR => Some(IpldBlock { codec, data: params.into() }),
            0 if params.is_empty() => None,
            _ => return Err(PrecompileError::InvalidInput),
        };
        system.send_raw(
            &address,
            method,
            params,
            TokenAmount::from(&value),
            Some(ctx.gas_limit),
            flags,
        )
    };

    // ------ Build Output -------

    let output = {
        // negative values are syscall/system errors
        // positive values are user/actor errors
        // success is 0
        let (exit_code, data) = match result {
            Err(syscall_err) => {
                let exit_code = U256::from(syscall_err.value());
                (exit_code.i256_neg(), None)
            }
            Ok(ret) => match ret {
                Err(exit) => {
                    // put error number from call into revert
                    let exit_code = U256::from(exit.exit_code.value());
                    // exit with any return data
                    (exit_code, exit.return_data)
                }
                Ok(ret) => (U256::zero(), ret),
            },
        };

        let ret_blk = data.unwrap_or(IpldBlock { codec: 0, data: vec![] });

        let mut output = Vec::with_capacity(4 * 32 + ret_blk.data.len());
        output.extend_from_slice(&exit_code.to_bytes());
        output.extend_from_slice(&U256::from(ret_blk.codec).to_bytes());
        output.extend_from_slice(&U256::from(output.len() + 32).to_bytes());
        output.extend_from_slice(&U256::from(ret_blk.data.len()).to_bytes());
        output.extend_from_slice(&ret_blk.data);
        // Pad out to the next increment of 32 bytes for solidity compatibility.
        let offset = output.len() % 32;
        if offset > 0 {
            output.resize(output.len() - offset + 32, 0);
        }
        output
    };

    Ok(output)
}
