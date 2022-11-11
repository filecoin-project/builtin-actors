use std::iter;

use fil_actors_runtime::{runtime::builtins::Type, AsActorError, EAM_ACTOR_ID};
use fvm_ipld_encoding::{strict_bytes, BytesDe, BytesSer};
use fvm_shared::address::{Address, Payload};
use interpreter::{address::EthAddress, system::load_bytecode};

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

/// Maximum allowed EVM bytecode size.
/// The contract code size limit is 24kB.
const MAX_CODE_SIZE: usize = 24 << 10;

pub const EVM_CONTRACT_REVERTED: ExitCode = ExitCode::new(27);

#[derive(FromPrimitive)]
#[repr(u64)]
pub enum Method {
    Constructor = METHOD_CONSTRUCTOR,
    InvokeContract = 2,
    GetBytecode = 3,
    GetStorageAt = 4,
    InvokeContractReadOnly = 5,
    InvokeContractDelegate = 6,
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

        if params.initcode.len() > MAX_CODE_SIZE {
            return Err(ActorError::illegal_argument(format!(
                "EVM byte code length ({}) is exceeding the maximum allowed of {MAX_CODE_SIZE}",
                params.initcode.len()
            )));
        } else if params.initcode.is_empty() {
            return Err(ActorError::illegal_argument("no initcode provided".into()));
        }

        let mut system = System::create(rt)?;

        // create a new execution context
        let mut exec_state = ExecutionState::new(
            params.creator,
            receiver_eth_addr,
            Method::Constructor as u64,
            Bytes::new(),
        );

        // identify bytecode valid jump destinations
        let initcode = Bytecode::new(params.initcode.into());

        // invoke the contract constructor
        let exec_status =
            execute(&initcode, &mut exec_state, &mut system).map_err(|e| match e {
                StatusCode::ActorError(e) => e,
                _ => ActorError::unspecified(format!("EVM execution error: {e:?}")),
            })?;

        // TODO this does not return revert data yet, but it has correct semantics.
        if exec_status.reverted {
            Err(ActorError::unchecked(EVM_CONTRACT_REVERTED, "constructor reverted".to_string()))
        } else if exec_status.status_code == StatusCode::Success {
            if exec_status.output_data.is_empty() {
                return Err(ActorError::unspecified(
                    "EVM constructor returned empty contract".to_string(),
                ));
            }
            // constructor ran to completion successfully and returned
            // the resulting bytecode.
            let contract_bytecode = exec_status.output_data;

            system.set_bytecode(&contract_bytecode)?;
            system.flush()
        } else if let StatusCode::ActorError(e) = exec_status.status_code {
            Err(e)
        } else {
            Err(ActorError::unspecified("EVM constructor failed".to_string()))
        }
    }

    pub fn invoke_contract<RT>(
        rt: &mut RT,
        method: u64,
        input_data: &[u8],
        readonly: bool,
        with_code: Option<Cid>,
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

        let mut system = System::load(rt, readonly).map_err(|e| {
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

        let mut exec_state = ExecutionState::new(
            caller_eth_addr,
            receiver_eth_addr,
            method,
            input_data.to_vec().into(),
        );

        let exec_status =
            execute(&bytecode, &mut exec_state, &mut system).map_err(|e| match e {
                StatusCode::ActorError(e) => e,
                _ => ActorError::unspecified(format!("EVM execution error: {e:?}")),
            })?;

        // TODO this does not return revert data yet, but it has correct semantics.
        if exec_status.reverted {
            return Err(ActorError::unchecked(
                EVM_CONTRACT_REVERTED,
                "contract reverted".to_string(),
            ));
        } else if exec_status.status_code == StatusCode::Success {
            system.flush()?;
        } else if let StatusCode::ActorError(e) = exec_status.status_code {
            return Err(e);
        } else {
            return Err(ActorError::unspecified(format!(
                "EVM contract invocation failed: status: {}",
                exec_status.status_code
            )));
        }

        if let Some(addr) = exec_status.selfdestroyed {
            rt.delete_actor(&addr)?
        }

        Ok(exec_status.output_data.to_vec())
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
        match FromPrimitive::from_u64(method) {
            Some(Method::Constructor) => {
                Self::constructor(rt, cbor::deserialize_params(params)?)?;
                Ok(RawBytes::default())
            }
            Some(Method::InvokeContract) => {
                let BytesDe(params) = params.deserialize()?;
                let value =
                    Self::invoke_contract(rt, Method::InvokeContract as u64, &params, false, None)?;
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
            Some(Method::InvokeContractReadOnly) => {
                let BytesDe(params) = params.deserialize()?;
                let value = Self::invoke_contract(
                    rt,
                    Method::InvokeContractReadOnly as u64,
                    &params,
                    true,
                    None,
                )?;
                Ok(RawBytes::serialize(BytesSer(&value))?)
            }
            Some(Method::InvokeContractDelegate) => {
                let params: DelegateCallParams = cbor::deserialize_params(params)?;
                let value = Self::invoke_contract(
                    rt,
                    Method::InvokeContractDelegate as u64,
                    &params.input,
                    params.readonly,
                    Some(params.code),
                )?;
                Ok(RawBytes::serialize(BytesSer(&value))?)
            }

            // Otherwise, we take the bytes as CBOR.
            None => Self::invoke_contract(rt, method, params, false, None).map(RawBytes::new),
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
    /// Whether the call is within a read only (static) call context
    pub readonly: bool,
}

#[derive(Serialize_tuple, Deserialize_tuple)]
pub struct GetStorageAtParams {
    pub storage_key: U256,
}
