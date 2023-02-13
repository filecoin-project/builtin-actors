use fil_actors_evm_shared::address::EthAddress;
use fil_actors_evm_shared::uints::U256;
use fil_actors_runtime::{actor_error, ActorError, AsActorError, EAM_ACTOR_ADDR, INIT_ACTOR_ADDR};
use fvm_ipld_blockstore::Blockstore;
use fvm_ipld_encoding::ipld_block::IpldBlock;
use fvm_ipld_encoding::{BytesDe, BytesSer};
use fvm_shared::address::Address;
use fvm_shared::econ::TokenAmount;
use fvm_shared::error::ExitCode;

use crate::interpreter::Outcome;
use crate::interpreter::{execute, Bytecode, ExecutionState, System};
use crate::reader::ValueReader;
use cid::Cid;
use fil_actors_runtime::runtime::{ActorCode, Runtime};
use fvm_shared::{MethodNum, METHOD_CONSTRUCTOR};
use num_derive::FromPrimitive;
use num_traits::FromPrimitive;

pub use types::*;

#[doc(hidden)]
pub mod ext;
pub mod interpreter;
pub(crate) mod reader;
mod state;
mod types;

pub use state::*;

#[cfg(feature = "fil-actor")]
fil_actors_runtime::wasm_trampoline!(EvmContractActor);

pub const EVM_CONTRACT_REVERTED: ExitCode = ExitCode::new(33);
pub const EVM_CONTRACT_INVALID_INSTRUCTION: ExitCode = ExitCode::new(34);
pub const EVM_CONTRACT_UNDEFINED_INSTRUCTION: ExitCode = ExitCode::new(35);
pub const EVM_CONTRACT_STACK_UNDERFLOW: ExitCode = ExitCode::new(36);
pub const EVM_CONTRACT_STACK_OVERFLOW: ExitCode = ExitCode::new(37);
pub const EVM_CONTRACT_ILLEGAL_MEMORY_ACCESS: ExitCode = ExitCode::new(38);
pub const EVM_CONTRACT_BAD_JUMPDEST: ExitCode = ExitCode::new(39);
pub const EVM_CONTRACT_SELFDESTRUCT_FAILED: ExitCode = ExitCode::new(40);

const EVM_MAX_RESERVED_METHOD: u64 = 1023;
pub const NATIVE_METHOD_SIGNATURE: &str = "handle_filecoin_method(uint64,uint64,bytes)";
pub const NATIVE_METHOD_SELECTOR: [u8; 4] = [0x86, 0x8e, 0x10, 0xc4];

const EVM_WORD_SIZE: usize = 32;

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
    Resurrect = 2,
    GetBytecode = 3,
    GetBytecodeHash = 4,
    GetStorageAt = 5,
    InvokeContractDelegate = 6,
    InvokeContract = frc42_dispatch::method_hash!("InvokeEVM"),
}

pub struct EvmContractActor;

/// Returns a tombstone for the currently executing message.
pub(crate) fn current_tombstone(rt: &impl Runtime) -> Tombstone {
    Tombstone { origin: rt.message().origin().id().unwrap(), nonce: rt.message().nonce() }
}

/// Returns true if the contract is "dead". A contract is dead if:
///
/// 1. It has a tombstone.
/// 2. It's tombstone is not from the current message execution (the nonce/origin don't match the
///    currently executing message).
///
/// Specifically, this lets us mark the contract as "self-destructed" but keep it alive until the
/// current top-level message finishes executing.
pub(crate) fn is_dead(rt: &impl Runtime, state: &State) -> bool {
    state.tombstone.map_or(false, |t| t != current_tombstone(rt))
}

fn load_bytecode(bs: &impl Blockstore, cid: &Cid) -> Result<Option<Bytecode>, ActorError> {
    let bytecode = bs
        .get(cid)
        .context_code(ExitCode::USR_NOT_FOUND, "failed to read bytecode")?
        .expect("bytecode not in state tree");
    if bytecode.is_empty() {
        Ok(None)
    } else {
        Ok(Some(Bytecode::new(bytecode)))
    }
}

fn initialize_evm_contract(
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
        return Err(ActorError::forbidden(format!(
            "contract {} doesn't have an eth address",
            receiver_fil_addr,
        )));
    }

    // If we have no code, save the state and return.
    if initcode.is_empty() {
        return system.flush();
    }

    // create a new execution context
    let value_received = system.rt.message().value_received();
    let mut exec_state = ExecutionState::new(caller, receiver_eth_addr, value_received, Vec::new());

    // identify bytecode valid jump destinations
    let initcode = Bytecode::new(initcode);

    // invoke the contract constructor
    let output = execute(&initcode, &mut exec_state, system)?;

    match output.outcome {
        Outcome::Return => {
            system.set_bytecode(&output.return_data)?;
            system.flush()
        }
        Outcome::Revert => Err(ActorError::unchecked_with_data(
            EVM_CONTRACT_REVERTED,
            "constructor reverted".to_string(),
            IpldBlock::serialize_cbor(&BytesSer(&output.return_data)).unwrap(),
        )),
    }
}

fn invoke_contract_inner<RT>(
    system: &mut System<RT>,
    input_data: Vec<u8>,
    bytecode_cid: &Cid,
    caller: &EthAddress,
    value_received: TokenAmount,
) -> Result<Vec<u8>, ActorError>
where
    RT: Runtime,
    RT::Blockstore: Clone,
{
    let bytecode = match load_bytecode(system.rt.store(), bytecode_cid)? {
        Some(bytecode) => bytecode,
        // an EVM contract with no code returns immediately
        None => return Ok(Vec::new()),
    };

    // Resolve the receiver's ethereum address.
    let receiver_fil_addr = system.rt.message().receiver();
    let receiver_eth_addr = system.resolve_ethereum_address(&receiver_fil_addr).unwrap();

    let mut exec_state =
        ExecutionState::new(*caller, receiver_eth_addr, value_received, input_data);

    let output = execute(&bytecode, &mut exec_state, system)?;

    match output.outcome {
        Outcome::Return => {
            system.flush()?;
            Ok(output.return_data.to_vec())
        }
        Outcome::Revert => Err(ActorError::unchecked_with_data(
            EVM_CONTRACT_REVERTED,
            "contract reverted".to_string(),
            IpldBlock::serialize_cbor(&BytesSer(&output.return_data)).unwrap(),
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

    pub fn resurrect<RT>(rt: &mut RT, params: ResurrectParams) -> Result<(), ActorError>
    where
        RT: Runtime,
        RT::Blockstore: Clone,
    {
        rt.validate_immediate_caller_is(&[EAM_ACTOR_ADDR])?;
        initialize_evm_contract(&mut System::resurrect(rt)?, params.creator, params.initcode.into())
    }

    pub fn invoke_contract_delegate<RT>(
        rt: &mut RT,
        params: DelegateCallParams,
    ) -> Result<Vec<u8>, ActorError>
    where
        RT: Runtime,
        RT::Blockstore: Clone,
    {
        rt.validate_immediate_caller_is(&[rt.message().receiver()])?;

        let mut system = System::load(rt).map_err(|e| {
            ActorError::unspecified(format!("failed to create execution abstraction layer: {e:?}"))
        })?;
        invoke_contract_inner(&mut system, params.input, &params.code, &params.caller, params.value)
    }

    pub fn invoke_contract<RT>(rt: &mut RT, input_data: Vec<u8>) -> Result<Vec<u8>, ActorError>
    where
        RT: Runtime,
        RT::Blockstore: Clone,
    {
        rt.validate_immediate_caller_accept_any()?;

        let mut system = System::load(rt).map_err(|e| {
            ActorError::unspecified(format!("failed to create execution abstraction layer: {e:?}"))
        })?;

        let bytecode_cid = match system.get_bytecode() {
            Some(bytecode_cid) => bytecode_cid,
            // an EVM contract with no code returns immediately
            None => return Ok(Vec::new()),
        };

        let received_value = system.rt.message().value_received();
        let caller = system.resolve_ethereum_address(&system.rt.message().caller()).unwrap();
        invoke_contract_inner(&mut system, input_data, &bytecode_cid, &caller, received_value)
    }

    pub fn handle_filecoin_method<RT>(
        rt: &mut RT,
        method: u64,
        args: Option<IpldBlock>,
    ) -> Result<Option<IpldBlock>, ActorError>
    where
        RT: Runtime,
        RT::Blockstore: Clone,
    {
        let params = args.unwrap_or(IpldBlock { codec: 0, data: vec![] });
        let input = handle_filecoin_method_input(method, params.codec, params.data.as_slice());
        let output = Self::invoke_contract(rt, input)?;
        handle_filecoin_method_output(&output)
    }

    /// Returns the contract's EVM bytecode, or `None` if the contract has been deleted (has called
    /// SELFDESTRUCT).
    pub fn bytecode(rt: &mut impl Runtime) -> Result<Option<Cid>, ActorError> {
        // Any caller can fetch the bytecode of a contract; this is now EXT* opcodes work.
        rt.validate_immediate_caller_accept_any()?;

        let state: State = rt.state()?;
        if is_dead(rt, &state) {
            Ok(None)
        } else {
            Ok(Some(state.bytecode))
        }
    }

    pub fn bytecode_hash(rt: &mut impl Runtime) -> Result<BytecodeHash, ActorError> {
        // Any caller can fetch the bytecode hash of a contract; this is where EXTCODEHASH gets it's value for EVM contracts.
        rt.validate_immediate_caller_accept_any()?;

        // return value must be either keccak("") or keccak(bytecode)
        let state: State = rt.state()?;
        if is_dead(rt, &state) {
            Ok(BytecodeHash::EMPTY)
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
    let static_args =
        [method, codec, EVM_WORD_SIZE as u64 * 3 /* start of params */, params.len() as u64];
    let total_words =
        static_args.len() + (params.len() / EVM_WORD_SIZE) + (params.len() % 32 > 0) as usize;
    let len = 4 + total_words * EVM_WORD_SIZE;
    let mut buf = Vec::with_capacity(len);
    buf.extend_from_slice(&NATIVE_METHOD_SELECTOR);
    for n in static_args {
        // Left-pad to 32 bytes, then be-encode the value.
        let encoded = n.to_be_bytes();
        buf.resize(buf.len() + (EVM_WORD_SIZE - encoded.len()), 0);
        buf.extend_from_slice(&encoded);
    }
    // Extend with the params, then right-pad with zeros.
    buf.extend_from_slice(params);
    buf.resize(len, 0);
    buf
}

/// Decode the response from "filecoin_native_method". We expect:
///
/// 1. The exit code (u32).
/// 2. The codec (u64).
/// 3. The data (bytes).
///
/// According to the solidity ABI.
fn handle_filecoin_method_output(output: &[u8]) -> Result<Option<IpldBlock>, ActorError> {
    // Short-circuit if empty.
    if output.is_empty() {
        return Ok(None);
    }
    let mut output = ValueReader::new(output);

    let exit_code: ExitCode =
        output.read_value().context_code(ExitCode::USR_SERIALIZATION, "exit code not a u32")?;
    let codec: u64 = output
        .read_value()
        .context_code(ExitCode::USR_SERIALIZATION, "returned codec not a u64")?;
    let len_offset: u32 = output
        .read_value()
        .context_code(ExitCode::USR_SERIALIZATION, "invalid return value offset")?;

    output.seek(len_offset as usize);
    let length: u32 = output
        .read_value()
        .context_code(ExitCode::USR_SERIALIZATION, "return length is too large")?;
    let return_data = output.read_padded(length as usize);

    let return_block = match codec {
        // Empty return values.
        0 if length == 0 => None,
        0 => {
            return Err(ActorError::serialization(format!(
                "codec 0 is only valid for empty returns, got a return value of length {length}"
            )));
        }
        // Supported codecs.
        fvm_ipld_encoding::CBOR => Some(IpldBlock { codec, data: return_data.into() }),
        // Everything else.
        _ => return Err(ActorError::serialization(format!("unsupported codec: {codec}"))),
    };

    if exit_code.is_success() {
        Ok(return_block)
    } else {
        Err(ActorError::unchecked_with_data(
            exit_code,
            "EVM contract explicitly exited with a non-zero exit code".to_string(),
            return_block,
        ))
    }
}

impl ActorCode for EvmContractActor {
    type Methods = Method;
    // TODO: Use actor_dispatch macros for this: https://github.com/filecoin-project/builtin-actors/issues/966
    fn invoke_method<RT>(
        rt: &mut RT,
        method: MethodNum,
        args: Option<IpldBlock>,
    ) -> Result<Option<IpldBlock>, ActorError>
    where
        RT: Runtime,
        RT::Blockstore: Clone,
    {
        match FromPrimitive::from_u64(method) {
            Some(Method::Constructor) => {
                Self::constructor(
                    rt,
                    args.with_context_code(ExitCode::USR_ILLEGAL_ARGUMENT, || {
                        "method expects arguments".to_string()
                    })?
                    .deserialize()?,
                )?;
                Ok(None)
            }
            Some(Method::InvokeContract) => {
                let params = match args {
                    None => vec![],
                    Some(p) => {
                        let BytesDe(p) = p.deserialize()?;
                        p
                    }
                };
                let value = Self::invoke_contract(rt, params)?;
                Ok(IpldBlock::serialize_cbor(&BytesSer(&value))?)
            }
            Some(Method::GetBytecode) => {
                let ret = Self::bytecode(rt)?;
                Ok(IpldBlock::serialize_dag_cbor(&ret)?)
            }
            Some(Method::GetBytecodeHash) => {
                let hash = Self::bytecode_hash(rt)?;
                Ok(IpldBlock::serialize_cbor(&hash)?)
            }
            Some(Method::GetStorageAt) => {
                let value = Self::storage_at(
                    rt,
                    args.with_context_code(ExitCode::USR_ILLEGAL_ARGUMENT, || {
                        "method expects arguments".to_string()
                    })?
                    .deserialize()?,
                )?;
                Ok(IpldBlock::serialize_cbor(&value)?)
            }
            Some(Method::InvokeContractDelegate) => {
                let params: DelegateCallParams = args
                    .with_context_code(ExitCode::USR_ILLEGAL_ARGUMENT, || {
                        "method expects arguments".to_string()
                    })?
                    .deserialize()?;
                let value = Self::invoke_contract_delegate(rt, params)?;
                Ok(IpldBlock::serialize_cbor(&BytesSer(&value))?)
            }
            Some(Method::Resurrect) => {
                Self::resurrect(
                    rt,
                    args.with_context_code(ExitCode::USR_ILLEGAL_ARGUMENT, || {
                        "method expects arguments".to_string()
                    })?
                    .deserialize()?,
                )?;
                Ok(None)
            }
            None if method > EVM_MAX_RESERVED_METHOD => {
                // We reserve all methods below EVM_MAX_RESERVED (<= 1023) method. This is a
                // _subset_ of those reserved by FRC0042.
                Self::handle_filecoin_method(rt, method, args)
            }
            None => Err(actor_error!(unhandled_message; "Invalid method")),
        }
    }
}
