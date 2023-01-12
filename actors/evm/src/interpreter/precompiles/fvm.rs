use fil_actors_runtime::runtime::{builtins::Type, Runtime};
use fvm_ipld_encoding::ipld_block::IpldBlock;
use fvm_shared::{address::Address, econ::TokenAmount, sys::SendFlags};
use num_traits::FromPrimitive;

use crate::interpreter::{
    instructions::call::CallKind,
    precompiles::{
        parameter::{read_right_pad, Parameter},
        NativeType,
    },
    System, U256,
};

use super::{parameter::U256Reader, PrecompileContext, PrecompileError, PrecompileResult};

/// Read right padded BE encoded low u64 ID address from a u256 word.
/// Returns variant of [`BuiltinType`] encoded as a u256 word.
/// Returns nothing inputs >2^65
pub(super) fn get_actor_type<RT: Runtime>(
    system: &mut System<RT>,
    input: &[u8],
    _: PrecompileContext,
) -> PrecompileResult {
    // should never panic, pad to 32 bytes then read exactly 32 bytes
    let id_bytes: [u8; 32] = read_right_pad(input, 32)[..32].as_ref().try_into().unwrap();
    let id = match Parameter::<u64>::try_from(&id_bytes) {
        Ok(id) => id.0,
        Err(_) => return Ok(Vec::new()),
    };

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
    let mut input_params = U256Reader::new(input);

    #[derive(num_derive::FromPrimitive)]
    #[repr(i32)]
    enum RandomnessType {
        Chain = 0,
        Beacon = 1,
    }

    let randomness_type = RandomnessType::from_i32(input_params.next_param_padded::<i32>()?);
    let personalization = input_params.next_param_padded::<i64>()?;
    let rand_epoch = input_params.next_param_padded::<i64>()?;
    let entropy_len = input_params.next_param_padded::<u32>()?;

    debug_assert_eq!(input_params.chunks_read(), 4);

    let entropy = read_right_pad(input_params.remaining_slice(), entropy_len as usize);

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
    let mut id_bytes = U256Reader::new(input);
    let id = id_bytes.next_param_padded::<u64>()?;

    let address = system.rt.lookup_delegated_address(id);
    let ab = match address {
        Some(a) => a.to_bytes(),
        None => Vec::new(),
    };
    Ok(ab)
}

/// Reads a FIL (i.e. f0xxx, f4x1xxx) encoded address
/// Resolves a FIL encoded address into an ID address
/// Returns BE encoded u256 (return will always be under 2^64). Empty array if nothing found or input length was larger 2^32.
pub(super) fn resolve_address<RT: Runtime>(
    system: &mut System<RT>,
    input: &[u8],
    _: PrecompileContext,
) -> PrecompileResult {
    let mut input_params = U256Reader::new(input);

    let len = input_params.next_param_padded::<u32>()? as usize;
    // pad right as needed
    let padded = read_right_pad(input_params.remaining_slice(), len);
    let addr = match Address::from_bytes(padded.get(..len).ok_or(PrecompileError::InternalErr)?) {
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

/// Errors:
///    TODO should just give 0s?
/// - `IncorrectInputSize` if offset is larger than total input length
/// - `InvalidInput` if supplied address bytes isnt a filecoin address
///
/// Returns:
///
/// `[int256 exit_code, uint codec, uint offset, uint size, []bytes <actor return value>]`
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
    // ----- Input Parameters -------

    if ctx.call_type != CallKind::DelegateCall {
        return Err(PrecompileError::CallForbidden);
    }

    let mut input_params = U256Reader::new(input);

    let method: u64 = input_params.next_param_padded()?;

    let value: U256 = input_params.next_padded().into();

    let flags: u64 = input_params.next_param_padded()?;
    let flags = SendFlags::from_bits(flags).ok_or(PrecompileError::InvalidInput)?;

    let codec: u64 = input_params.next_param_padded()?;

    let send_data_size = input_params.next_param_padded::<u32>()? as usize;
    let address_size = input_params.next_param_padded::<u32>()? as usize;

    // ------ Begin Call -------

    let result = {
        let start = input_params.remaining_slice();
        let bytes = read_right_pad(start, send_data_size + address_size);

        let input_data = &bytes[..send_data_size];
        let address = &bytes[send_data_size..send_data_size + address_size];
        let address = Address::from_bytes(address).map_err(|_| PrecompileError::InvalidInput)?;

        // TODO only CBOR or "nothing" for now
        let params = match codec {
            fvm_ipld_encoding::DAG_CBOR => Some(IpldBlock { codec, data: input_data.into() }),
            0 if input_data.is_empty() => None,
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

        const NUM_OUTPUT_PARAMS: u32 = 4;

        let ret_blk = data.unwrap_or(IpldBlock { codec: 0, data: vec![] });
        let offset = NUM_OUTPUT_PARAMS * 32;

        let mut output = Vec::with_capacity(NUM_OUTPUT_PARAMS as usize * 32 + ret_blk.data.len());
        output.extend_from_slice(&exit_code.to_bytes());
        output.extend_from_slice(&U256::from(ret_blk.codec).to_bytes());
        output.extend_from_slice(&U256::from(offset).to_bytes());
        output.extend_from_slice(&U256::from(ret_blk.data.len()).to_bytes());
        // NOTE:
        // we dont pad out to 32 bytes here, the idea being that users will already be in the "everything is bytes" mode
        // and will want re-pack align and whatever else by themselves
        output.extend_from_slice(&ret_blk.data);
        output
    };

    Ok(output)
}
