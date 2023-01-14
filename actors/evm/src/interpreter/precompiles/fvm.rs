use crate::EVM_MAX_RESERVED_METHOD;
use fil_actors_runtime::runtime::{builtins::Type, Runtime};
use fvm_ipld_encoding::ipld_block::IpldBlock;
use fvm_shared::{address::Address, econ::TokenAmount, sys::SendFlags};
use num_traits::FromPrimitive;

use crate::interpreter::{instructions::call::CallKind, precompiles::NativeType, System, U256};

use super::{parameter::ParameterReader, PrecompileContext, PrecompileError, PrecompileResult};

/// Read right padded BE encoded low u64 ID address from a u256 word.
/// Returns variant of [`BuiltinType`] encoded as a u256 word.
/// Returns nothing inputs >2^65
pub(super) fn get_actor_type<RT: Runtime>(
    system: &mut System<RT>,
    input: &[u8],
    _: PrecompileContext,
) -> PrecompileResult {
    let mut reader = ParameterReader::new(input);
    let id: u64 = reader.read_param()?;

    // resolve type from code CID
    let builtin_type = system
        .rt
        .get_actor_code_cid(&id)
        .and_then(|cid| system.rt.resolve_builtin_actor_type(&cid));

    let builtin_type = match builtin_type {
        Some(t) => match t {
            Type::Account | Type::EthAccount => NativeType::Account,
            Type::Placeholder => NativeType::Placeholder,
            Type::EVM => NativeType::EVMContract,
            Type::Miner => NativeType::StorageProvider,
            // Others
            Type::PaymentChannel | Type::Multisig => NativeType::OtherTypes,
            // Singletons (this should be caught earlier, but we are being exhaustive)
            Type::Market
            | Type::Power
            | Type::Init
            | Type::Cron
            | Type::Reward
            | Type::VerifiedRegistry
            | Type::DataCap
            | Type::EAM
            | Type::System => NativeType::System,
        },
        None => NativeType::NonExistent,
    };

    Ok(builtin_type.word_vec())
}

/// !! DISABLED !!
///
/// Params:
///
/// | Param            | Value                     |
/// |------------------|---------------------------|
/// | randomness_type  | U256 - low i32: `Chain`(0) OR `Beacon`(1) |
/// | personalization  | U256 - low i64             |
/// | randomness_epoch | U256 - low i64             |
/// | entropy_length   | U256 - low u32             |
/// | entropy          | input\[32..] (right padded)|
///
/// any bytes in between values are ignored
///
/// Returns empty array if invalid randomness type
/// Errors if unable to fetch randomness
#[allow(unused)]
pub(super) fn get_randomness<RT: Runtime>(
    system: &mut System<RT>,
    input: &[u8],
    _: PrecompileContext,
) -> PrecompileResult {
    let mut input_params = ParameterReader::new(input);

    #[derive(num_derive::FromPrimitive)]
    #[repr(i32)]
    enum RandomnessType {
        Chain = 0,
        Beacon = 1,
    }

    let randomness_type = RandomnessType::from_i32(input_params.read_param::<i32>()?);
    let personalization = input_params.read_param::<i64>()?;
    let rand_epoch = input_params.read_param::<i64>()?;
    let entropy_len = input_params.read_param::<u32>()? as usize;

    let entropy = input_params.read_padded(entropy_len);

    let randomness = match randomness_type {
        Some(RandomnessType::Chain) => system
            .rt
            .user_get_randomness_from_chain(personalization, rand_epoch, &entropy)
            .map(|a| a.to_vec()),
        Some(RandomnessType::Beacon) => system
            .rt
            .user_get_randomness_from_beacon(personalization, rand_epoch, &entropy)
            .map(|a| a.to_vec()),
        None => Ok(Vec::new()),
    };

    randomness.map_err(|_| PrecompileError::InvalidInput)
}

/// Read BE encoded low u64 ID address from a u256 word
/// Looks up and returns the encoded f4 addresses of an ID address. Empty array if not found or `InvalidInput` input was larger 2^64.
pub(super) fn lookup_delegated_address<RT: Runtime>(
    system: &mut System<RT>,
    input: &[u8],
    _: PrecompileContext,
) -> PrecompileResult {
    let mut id_bytes = ParameterReader::new(input);
    let id = id_bytes.read_param::<u64>()?;

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
    let addr = match Address::from_bytes(&input) {
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
/// bytes address
/// u64   method
/// u256  value
/// u64   flags (1 for read-only, 0 otherwise)
/// u64   codec (0x71 for "dag-cbor", or `0` for "nothing")
/// bytes params (must be empty if the codec is 0x0)
/// ```
///
/// Returns (also solidity ABI encoded):
///
/// `[int256 exit_code, uint codec, uint offset, uint size, []bytes <actor return value>]`
/// ```
/// i256  exit_code
/// u64   codec
/// bytes return_value
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
/// u64   actor_id
/// u64   method
/// u256  value
/// u64   flags (1 for read-only, 0 otherwise)
/// u64   codec (0x71 for "dag-cbor", or `0` for "nothing")
/// bytes params (must be empty if the codec is 0x0)
/// ```
///
/// Returns (also solidity ABI encoded):
///
/// `[int256 exit_code, uint codec, uint offset, uint size, []bytes <actor return value>]`
/// ```
/// i256  exit_code
/// u64   codec
/// bytes return_value
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

    let mut input_params = ParameterReader::new(input);

    let method: u64 = input_params.read_param()?;

    let value: U256 = input_params.read_param()?;

    let flags: u64 = input_params.read_param()?;
    let flags = SendFlags::from_bits(flags).ok_or(PrecompileError::InvalidInput)?;

    let codec: u64 = input_params.read_param()?;

    let params_off: u32 = input_params.read_param()?;
    let id_or_addr_off: u64 = input_params.read_param()?;

    input_params.seek(params_off.try_into()?);
    let params_len: u32 = input_params.read_param()?;
    let params = input_params.read_padded(params_len.try_into()?);

    let address = if by_id {
        Address::new_id(id_or_addr_off)
    } else {
        input_params.seek(id_or_addr_off.try_into()?);
        let addr_len: u32 = input_params.read_param()?;
        let addr_bytes = input_params
            .read_padded(addr_len.try_into().map_err(|_| PrecompileError::InvalidInput)?);
        Address::from_bytes(&addr_bytes).map_err(|_| PrecompileError::InvalidInput)?
    };

    if method <= EVM_MAX_RESERVED_METHOD {
        return Err(PrecompileError::InvalidInput);
    }

    // ------ Begin Call -------

    let result = {
        // TODO only CBOR or "nothing" for now
        let params = match codec {
            fvm_ipld_encoding::DAG_CBOR => Some(IpldBlock { codec, data: params.into() }),
            0 if params.is_empty() => None,
            _ => return Err(PrecompileError::InvalidInput),
        };
        system.send(&address, method, params, TokenAmount::from(&value), Some(ctx.gas_limit), flags)
    };

    // ------ Build Output -------

    let output = {
        // negative values are syscall errors
        // positive values are user/actor errors
        // success is 0
        let (exit_code, data) = match result {
            Err(mut ae) => {
                // TODO handle revert
                // TODO https://github.com/filecoin-project/ref-fvm/issues/1020
                // put error number from call into revert
                let exit_code = U256::from(ae.exit_code().value());

                // no return only exit code
                (exit_code, ae.take_data())
            }
            Ok(ret) => (U256::zero(), ret),
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
            output.resize(output.len() - offset - 32, 0);
        }
        output
    };

    Ok(output)
}
