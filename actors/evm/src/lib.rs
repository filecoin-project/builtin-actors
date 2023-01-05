use fil_actors_runtime::{actor_error, AsActorError, EAM_ACTOR_ADDR, INIT_ACTOR_ADDR};
use fvm_ipld_encoding::ipld_block::IpldBlock;
use fvm_ipld_encoding::{strict_bytes, BytesDe, BytesSer};
use fvm_shared::address::Address;
use fvm_shared::crypto::hash::SupportedHashes;
use fvm_shared::error::ExitCode;
use interpreter::instructions::ext::EMPTY_EVM_HASH;
use interpreter::{address::EthAddress, system::load_bytecode};
use multihash::Multihash;

use crate::interpreter::output::Outcome;

pub mod interpreter;
mod state;

use {
    crate::interpreter::{execute, Bytecode, ExecutionState, StatusCode, System, U256},
    bytes::Bytes,
    cid::Cid,
    fil_actors_runtime::{
        runtime::{ActorCode, Runtime},
        ActorError,
    },
    fvm_ipld_encoding::tuple::*,
    fvm_ipld_encoding::RawBytes,
    fvm_shared::{MethodNum, METHOD_CONSTRUCTOR},
    num_derive::FromPrimitive,
    num_traits::FromPrimitive,
};

pub use state::*;

#[cfg(feature = "fil-actor")]
fil_actors_runtime::wasm_trampoline!(EvmContractActor);

pub const EVM_CONTRACT_REVERTED: ExitCode = ExitCode::new(33);
pub const EVM_CONTRACT_EXECUTION_ERROR: ExitCode = ExitCode::new(34);

const EVM_MAX_RESERVED_METHOD: u64 = 1023;
pub const NATIVE_METHOD_SIGNATURE: &str = "handle_filecoin_method(uint64,uint64,bytes)";
pub const NATIVE_METHOD_SELECTOR: [u8; 4] = [0x86, 0x8e, 0x10, 0xc4];

#[test]
fn test_method_selector() {
    // We could just _generate_ this method selector with a proc macro, but this is easier.
    use cid::multihash::MultihashDigest;
    let hash = cid::multihash::Code::Keccak256.digest(NATIVE_METHOD_SIGNATURE.as_bytes());
    let computed_selector = &hash.digest()[..4];
    assert_eq!(computed_selector, NATIVE_METHOD_SELECTOR);
}

#[derive(FromPrimitive)]
#[repr(u64)]
pub enum Method {
    Constructor = METHOD_CONSTRUCTOR,
    // TODO: Do we want to use ExportedNums for all of these, per FRC-42?
    InvokeContract = 2,
    GetBytecode = 3,
    GetStorageAt = 4,
    InvokeContractDelegate = 5,
    GetBytecodeHash = 6,
    Resurrect = 7,
}

pub struct EvmContractActor;

/// Returns a tombstone for the currently executing message.
pub(crate) fn current_tombstone(rt: &impl Runtime) -> Tombstone {
    Tombstone { origin: rt.message().origin().id().unwrap(), nonce: rt.message().nonce() }
}

/// Returns true if the contract is "dead". A contract is dead if:
///
/// 1. It has a tombstone.
/// 2. It's tombstone is not from the current message execution.
pub(crate) fn is_dead(rt: &impl Runtime, state: &State) -> bool {
    state.tombstone.map_or(false, |t| t != current_tombstone(rt))
}

pub fn initialize_evm_contract(
    system: &mut System<impl Runtime>,
    caller: EthAddress,
    initcode: Vec<u8>,
) -> Result<(), ActorError> {
    // Lookup our Ethereum address.
    let receiver_fil_addr = system.rt.message().receiver();
    let receiver_eth_addr = system.resolve_ethereum_address(&receiver_fil_addr).context_code(
        ExitCode::USR_ASSERTION_FAILED,
        "failed to resolve the contracts ETH address",
    )?;

    // Make sure we have an actual Ethereum address (assigned by the EAM). This is how we make sure
    // an EVM actor may only be constructed by the EAM.
    if receiver_eth_addr.as_id().is_some() {
        return Err(ActorError::assertion_failed(format!(
            "contract {} doesn't have an eth address",
            receiver_fil_addr,
        )));
    }

    // If we have no code, save the state and return.
    if initcode.is_empty() {
        return system.flush();
    }

    // create a new execution context
    let mut exec_state = ExecutionState::new(caller, receiver_eth_addr, Bytes::new());

    // identify bytecode valid jump destinations
    let initcode = Bytecode::new(initcode);

    // invoke the contract constructor
    let output = execute(&initcode, &mut exec_state, system).map_err(|e| match e {
        StatusCode::ActorError(e) => e,
        _ => ActorError::unspecified(format!("EVM execution error: {e:?}")),
    })?;

    match output.outcome {
        Outcome::Return => {
            system.set_bytecode(&output.return_data)?;
            system.flush()
        }
        Outcome::Revert => Err(ActorError::unchecked_with_data(
            EVM_CONTRACT_REVERTED,
            "constructor reverted".to_string(),
            RawBytes::serialize(BytesSer(&output.return_data)).unwrap(),
        )),
    }
}

impl EvmContractActor {
    pub fn constructor<RT>(rt: &mut RT, params: ConstructorParams) -> Result<(), ActorError>
    where
        RT: Runtime,
        RT::Blockstore: Clone,
    {
        rt.validate_immediate_caller_is(&[INIT_ACTOR_ADDR])?;
        initialize_evm_contract(&mut System::create(rt)?, params.creator, params.initcode.into())
    }

    pub fn resurrect<RT>(rt: &mut RT, params: ConstructorParams) -> Result<(), ActorError>
    where
        RT: Runtime,
        RT::Blockstore: Clone,
    {
        rt.validate_immediate_caller_is(&[EAM_ACTOR_ADDR])?;
        initialize_evm_contract(&mut System::resurrect(rt)?, params.creator, params.initcode.into())
    }

    pub fn invoke_contract<RT>(
        rt: &mut RT,
        input_data: &[u8],
        with_code: Option<Cid>,
        with_caller: Option<EthAddress>,
    ) -> Result<Vec<u8>, ActorError>
    where
        RT: Runtime,
        RT::Blockstore: Clone,
    {
        if with_code.is_some() {
            rt.validate_immediate_caller_is(&[rt.message().receiver()])?;
        } else {
            rt.validate_immediate_caller_accept_any()?;
        }

        let mut system = System::load(rt).map_err(|e| {
            ActorError::unspecified(format!("failed to create execution abstraction layer: {e:?}"))
        })?;

        let bytecode = match match with_code {
            Some(cid) => load_bytecode(system.rt.store(), &cid),
            None => system.load_bytecode(),
        }? {
            Some(bytecode) => bytecode,
            // an EVM contract with no code returns immediately
            None => return Ok(Vec::new()),
        };

        // Use passed Eth address (from delegate call).
        // Otherwise resolve the caller's ethereum address. If the caller doesn't have one, the caller's Eth encoded ID is used instead.
        let caller_eth_addr = match with_caller {
            Some(addr) => addr,
            None => system.resolve_ethereum_address(&system.rt.message().caller()).unwrap(),
        };

        // Resolve the receiver's ethereum address.
        let receiver_fil_addr = system.rt.message().receiver();
        let receiver_eth_addr = system.resolve_ethereum_address(&receiver_fil_addr).unwrap();

        let mut exec_state =
            ExecutionState::new(caller_eth_addr, receiver_eth_addr, input_data.to_vec().into());

        let output = execute(&bytecode, &mut exec_state, &mut system).map_err(|e| match e {
            StatusCode::ActorError(e) => e,
            _ => ActorError::unspecified(format!("EVM execution error: {e:?}")),
        })?;

        match output.outcome {
            Outcome::Return => {
                system.flush()?;
                Ok(output.return_data.to_vec())
            }
            Outcome::Revert => Err(ActorError::unchecked_with_data(
                EVM_CONTRACT_REVERTED,
                "contract reverted".to_string(),
                RawBytes::serialize(BytesSer(&output.return_data)).unwrap(),
            )),
        }
    }

    pub fn handle_filecoin_method<RT>(
        rt: &mut RT,
        method: u64,
        args: Option<IpldBlock>,
    ) -> Result<Vec<u8>, ActorError>
    where
        RT: Runtime,
        RT::Blockstore: Clone,
    {
        let params = args.unwrap_or(IpldBlock { codec: 0, data: vec![] });
        let input = handle_filecoin_method_input(method, params.codec, params.data.as_slice());
        Self::invoke_contract(rt, &input, None, None)
    }

    pub fn bytecode(rt: &mut impl Runtime) -> Result<Cid, ActorError> {
        // Any caller can fetch the bytecode of a contract; this is now EXT* opcodes work.
        rt.validate_immediate_caller_accept_any()?;

        let state: State = rt.state()?;
        if is_dead(rt, &state) {
            // TODO: to return the "empty bytecode" cid, we'd need to actually write the empty
            // bytecode. Otherwise, it's not reachable.
            // Or we could implement https://github.com/filecoin-project/ref-fvm/issues/1358.
            // Finally, we could just return an error and let the caller deal with it?
            todo!("non-trivial?");
        } else {
            Ok(state.bytecode)
        }
    }

    pub fn bytecode_hash(rt: &mut impl Runtime) -> Result<multihash::Multihash, ActorError> {
        // Any caller can fetch the bytecode hash of a contract; this is where EXTCODEHASH gets it's value for EVM contracts.
        rt.validate_immediate_caller_accept_any()?;

        // return value must be either keccak("") or keccak(bytecode)
        let state: State = rt.state()?;
        if is_dead(rt, &state) {
            Ok(Multihash::wrap(SupportedHashes::Keccak256 as u64, &EMPTY_EVM_HASH).unwrap())
        } else {
            Ok(state.bytecode_hash)
        }
    }

    pub fn storage_at<RT>(rt: &mut RT, params: GetStorageAtParams) -> Result<U256, ActorError>
    where
        RT: Runtime,
        RT::Blockstore: Clone,
    {
        // This method cannot be called on-chain; other on-chain logic should not be able to
        // access arbitrary storage keys from a contract.
        rt.validate_immediate_caller_is([&Address::new_id(0)])?;

        // If the contract is dead, this will always return "0".
        System::load(rt)?
            .get_storage(params.storage_key)
            .context_code(ExitCode::USR_ASSERTION_FAILED, "failed to get storage key")
    }
}

/// Format "filecoin_native_method" input parameters.
fn handle_filecoin_method_input(method: u64, codec: u64, params: &[u8]) -> Vec<u8> {
    let static_args = [method, codec, 32 * 3 /* start of params */, params.len() as u64];
    let total_words = static_args.len() + (params.len() / 32) + (params.len() % 32 > 0) as usize;
    let len = 4 + total_words * 32;
    let mut buf = Vec::with_capacity(len);
    buf.extend_from_slice(&NATIVE_METHOD_SELECTOR);
    for n in static_args {
        // Left-pad to 32 bytes, then be-encode the value.
        let encoded = n.to_be_bytes();
        buf.resize(buf.len() + (32 - encoded.len()), 0);
        buf.extend_from_slice(&encoded);
    }
    // Extend with the params, then right-pad with zeros.
    buf.extend_from_slice(params);
    buf.resize(len, 0);
    buf
}

impl ActorCode for EvmContractActor {
    type Methods = Method;
    // TODO: Use actor_dispatch macros for this: https://github.com/filecoin-project/builtin-actors/issues/966
    fn invoke_method<RT>(
        rt: &mut RT,
        method: MethodNum,
        args: Option<IpldBlock>,
    ) -> Result<RawBytes, ActorError>
    where
        RT: Runtime,
        RT::Blockstore: Clone,
    {
        // We reserve all methods below EVM_MAX_RESERVED (<= 1023) method. This is a _subset_ of
        // those reserved by FRC0042.
        if method > EVM_MAX_RESERVED_METHOD {
            return Self::handle_filecoin_method(rt, method, args).map(RawBytes::new);
        }

        match FromPrimitive::from_u64(method) {
            Some(Method::Constructor) => {
                Self::constructor(
                    rt,
                    args.with_context_code(ExitCode::USR_ILLEGAL_ARGUMENT, || {
                        "method expects arguments".to_string()
                    })?
                    .deserialize()?,
                )?;
                Ok(RawBytes::default())
            }
            Some(Method::InvokeContract) => {
                let params = match args {
                    None => vec![],
                    Some(p) => {
                        let BytesDe(p) = p.deserialize()?;
                        p
                    }
                };
                let value = Self::invoke_contract(rt, &params, None, None)?;
                Ok(RawBytes::serialize(BytesSer(&value))?)
            }
            Some(Method::GetBytecode) => {
                let cid = Self::bytecode(rt)?;
                Ok(RawBytes::serialize(cid)?)
            }
            Some(Method::GetBytecodeHash) => {
                let multihash = Self::bytecode_hash(rt)?;
                Ok(RawBytes::serialize(multihash)?)
            }
            Some(Method::GetStorageAt) => {
                let value = Self::storage_at(
                    rt,
                    args.with_context_code(ExitCode::USR_ILLEGAL_ARGUMENT, || {
                        "method expects arguments".to_string()
                    })?
                    .deserialize()?,
                )?;
                Ok(RawBytes::serialize(value)?)
            }
            Some(Method::InvokeContractDelegate) => {
                let params: DelegateCallParams = args
                    .with_context_code(ExitCode::USR_ILLEGAL_ARGUMENT, || {
                        "method expects arguments".to_string()
                    })?
                    .deserialize()?;
                let value = Self::invoke_contract(
                    rt,
                    &params.input,
                    Some(params.code),
                    Some(params.caller),
                )?;
                Ok(RawBytes::serialize(BytesSer(&value))?)
            }
            Some(Method::Resurrect) => {
                Self::resurrect(
                    rt,
                    args.with_context_code(ExitCode::USR_ILLEGAL_ARGUMENT, || {
                        "method expects arguments".to_string()
                    })?
                    .deserialize()?,
                )?;
                Ok(RawBytes::default())
            }
            None => Err(actor_error!(unhandled_message; "Invalid method")),
        }
    }
}

#[derive(Serialize_tuple, Deserialize_tuple)]
pub struct ConstructorParams {
    /// The actor's "creator" (specified by the EAM).
    pub creator: EthAddress,
    /// The initcode that will construct the new EVM actor.
    pub initcode: RawBytes,
}

pub type ResurrectParams = ConstructorParams;

#[derive(Serialize_tuple, Deserialize_tuple)]
pub struct DelegateCallParams {
    pub code: Cid,
    /// The contract invocation parameters
    #[serde(with = "strict_bytes")]
    pub input: Vec<u8>,
    /// The original caller's Eth address.
    pub caller: EthAddress,
}

#[derive(Serialize_tuple, Deserialize_tuple)]
pub struct GetStorageAtParams {
    pub storage_key: U256,
}
