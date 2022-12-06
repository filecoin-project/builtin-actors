use std::iter;

use fil_actors_runtime::{actor_error, runtime::builtins::Type, AsActorError, EAM_ACTOR_ID};
use fvm_ipld_encoding::{strict_bytes, BytesDe, BytesSer, DAG_CBOR};
use fvm_shared::address::{Address, Payload};
use interpreter::{address::EthAddress, system::load_bytecode};

use crate::interpreter::output::Outcome;

pub mod interpreter;
mod state;

use {
    crate::interpreter::{execute, Bytecode, ExecutionState, StatusCode, System, U256},
    crate::state::State,
    bytes::Bytes,
    cid::Cid,
    fil_actors_runtime::{
        cbor,
        runtime::{ActorCode, Runtime},
        ActorError,
    },
    fvm_ipld_encoding::tuple::*,
    fvm_ipld_encoding::RawBytes,
    fvm_shared::error::*,
    fvm_shared::{MethodNum, METHOD_CONSTRUCTOR},
    num_derive::FromPrimitive,
    num_traits::FromPrimitive,
};

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
    InvokeContract = 2,
    GetBytecode = 3,
    GetStorageAt = 4,
    InvokeContractDelegate = 5,
    InvokeContractTransfer = 6,
}

pub struct EvmContractActor;
impl EvmContractActor {
    pub fn constructor<RT>(rt: &mut RT, params: ConstructorParams) -> Result<(), ActorError>
    where
        RT: Runtime,
        RT::Blockstore: Clone,
    {
        // TODO ideally we would be checking that we are constructed by the EAM actor,
        //   but instead we check for init and then assert that we have a delegated address.
        //   https://github.com/filecoin-project/ref-fvm/issues/746
        // rt.validate_immediate_caller_is(vec![&EAM_ACTOR_ADDR])?;
        rt.validate_immediate_caller_type(iter::once(&Type::Init))?;

        // Assert we are constructed with a delegated address from the EAM
        let receiver = rt.message().receiver();
        let delegated_addr = rt.lookup_address(receiver.id().unwrap()).ok_or_else(|| {
            ActorError::assertion_failed(format!(
                "EVM actor {} created without a delegated address",
                receiver
            ))
        })?;
        let delegated_addr = match delegated_addr.payload() {
            Payload::Delegated(delegated) if delegated.namespace() == EAM_ACTOR_ID => {
                // sanity check
                assert_eq!(delegated.subaddress().len(), 20);
                Ok(*delegated)
            }
            _ => Err(ActorError::assertion_failed(format!(
                "EVM actor with delegated address {} created not namespaced to the EAM {}",
                delegated_addr, EAM_ACTOR_ID,
            ))),
        }?;
        let receiver_eth_addr = {
            let subaddr: [u8; 20] = delegated_addr.subaddress().try_into().map_err(|_| {
                ActorError::assertion_failed(format!(
                    "expected 20 byte EVM address, found {} bytes",
                    delegated_addr.subaddress().len()
                ))
            })?;
            EthAddress(subaddr)
        };

        let mut system = System::create(rt)?;
        // If we have no code, save the state and return.
        if params.initcode.is_empty() {
            return system.flush();
        }

        // create a new execution context
        let mut exec_state = ExecutionState::new(params.creator, receiver_eth_addr, Bytes::new());

        // identify bytecode valid jump destinations
        let initcode = Bytecode::new(params.initcode.into());

        // invoke the contract constructor
        let output = execute(&initcode, &mut exec_state, &mut system).map_err(|e| match e {
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
            Outcome::Delete => Ok(()),
        }
    }

    pub fn invoke_contract<RT>(
        rt: &mut RT,
        input_data: &[u8],
        with_code: Option<Cid>,
        restricted: bool,
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

        let mut system = System::load(rt, restricted).map_err(|e| {
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

        // Resolve the caller's ethereum address. If the caller doesn't have one, the caller's ID is used instead.
        let caller_fil_addr = system.rt.message().caller();
        let caller_eth_addr = system.resolve_ethereum_address(&caller_fil_addr).unwrap();

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
            Outcome::Delete => Ok(Vec::new()),
        }
    }

    pub fn handle_filecoin_method<RT>(
        rt: &mut RT,
        method: u64,
        codec: u64,
        params: &[u8],
    ) -> Result<Vec<u8>, ActorError>
    where
        RT: Runtime,
        RT::Blockstore: Clone,
    {
        let input = handle_filecoin_method_input(method, codec, params);
        Self::invoke_contract(rt, &input, None, false)
    }

    pub fn bytecode(rt: &mut impl Runtime) -> Result<Cid, ActorError> {
        // Any caller can fetch the bytecode of a contract; this is now EXT* opcodes work.
        rt.validate_immediate_caller_accept_any()?;

        let state: State = rt.state()?;
        Ok(state.bytecode)
    }

    pub fn storage_at<RT>(rt: &mut RT, params: GetStorageAtParams) -> Result<U256, ActorError>
    where
        RT: Runtime,
        RT::Blockstore: Clone,
    {
        // This method cannot be called on-chain; other on-chain logic should not be able to
        // access arbitrary storage keys from a contract.
        rt.validate_immediate_caller_is([&Address::new_id(0)])?;

        System::load(rt, true)?
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
    fn invoke_method<RT>(
        rt: &mut RT,
        method: MethodNum,
        params: &RawBytes,
    ) -> Result<RawBytes, ActorError>
    where
        RT: Runtime,
        RT::Blockstore: Clone,
    {
        // We reserve all methods below EVM_MAX_RESERVED (<= 1023) method. This is a _subset_ of
        // those reserved by FRC0042.
        if method > EVM_MAX_RESERVED_METHOD {
            // FIXME: we need the actual codec.
            // See https://github.com/filecoin-project/ref-fvm/issues/987
            let codec = if params.is_empty() { 0 } else { DAG_CBOR };
            return Self::handle_filecoin_method(rt, method, codec, params).map(RawBytes::new);
        }

        match FromPrimitive::from_u64(method) {
            Some(Method::Constructor) => {
                Self::constructor(rt, cbor::deserialize_params(params)?)?;
                Ok(RawBytes::default())
            }
            Some(Method::InvokeContract) => {
                let BytesDe(params) = params.deserialize()?;
                let value = Self::invoke_contract(rt, &params, None, false)?;
                Ok(RawBytes::serialize(BytesSer(&value))?)
            }
            Some(Method::GetBytecode) => {
                let cid = Self::bytecode(rt)?;
                Ok(RawBytes::serialize(cid)?)
            }
            Some(Method::GetStorageAt) => {
                let value = Self::storage_at(rt, cbor::deserialize_params(params)?)?;
                Ok(RawBytes::serialize(value)?)
            }
            Some(Method::InvokeContractDelegate) => {
                let params: DelegateCallParams = cbor::deserialize_params(params)?;
                let value = Self::invoke_contract(rt, &params.input, Some(params.code), false)?;
                Ok(RawBytes::serialize(BytesSer(&value))?)
            }
            Some(Method::InvokeContractTransfer) => {
                let BytesDe(params) = params.deserialize()?;
                let value = Self::invoke_contract(rt, &params, None, true)?;
                Ok(RawBytes::serialize(BytesSer(&value))?)
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

#[derive(Serialize_tuple, Deserialize_tuple)]
pub struct DelegateCallParams {
    pub code: Cid,
    /// The contract invocation parameters
    #[serde(with = "strict_bytes")]
    pub input: Vec<u8>,
}

#[derive(Serialize_tuple, Deserialize_tuple)]
pub struct GetStorageAtParams {
    pub storage_key: U256,
}
