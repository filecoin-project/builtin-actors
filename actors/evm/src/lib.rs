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
        ActorDowncast, ActorError,
    },
    fvm_ipld_blockstore::Blockstore,
    fvm_ipld_encoding::tuple::*,
    fvm_ipld_encoding::RawBytes,
    fvm_ipld_hamt::Hamt,
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
}

pub struct EvmContractActor;
impl EvmContractActor {
    pub fn constructor<BS, RT>(rt: &mut RT, params: ConstructorParams) -> Result<(), ActorError>
    where
        BS: Blockstore + Clone,
        RT: Runtime<BS>,
    {
        rt.validate_immediate_caller_accept_any()?;

        if params.bytecode.len() > MAX_CODE_SIZE {
            return Err(ActorError::illegal_argument(format!(
                "EVM byte code length ({}) is exceeding the maximum allowed of {MAX_CODE_SIZE}",
                params.bytecode.len()
            )));
        }

        if params.bytecode.is_empty() {
            return Err(ActorError::illegal_argument("no bytecode provided".into()));
        }

        // create an empty storage HAMT to pass it down for execution.
        let mut hamt = Hamt::<_, U256, U256>::new(rt.store().clone());

        // create an instance of the platform abstraction layer -- note: do we even need this?
        let mut system = System::new(rt, &mut hamt).map_err(|e| {
            ActorError::unspecified(format!("failed to create execution abstraction layer: {e:?}"))
        })?;

        // create a new execution context
        let mut exec_state = ExecutionState::new(
            Method::Constructor as u64,
            Bytes::copy_from_slice(&params.input_data)
        );

        // identify bytecode valid jump destinations
        let bytecode = Bytecode::new(&params.bytecode)
            .map_err(|e| ActorError::unspecified(format!("failed to parse bytecode: {e:?}")))?;

        // invoke the contract constructor
        let exec_status =
            execute(&bytecode, &mut exec_state, &mut system.reborrow()).map_err(|e| match e {
                StatusCode::ActorError(e) => e,
                _ => ActorError::unspecified(format!("EVM execution error: {e:?}")),
            })?;

        // TODO this does not return revert data yet, but it has correct semantics.
        if exec_status.reverted {
            Err(ActorError::unchecked(EVM_CONTRACT_REVERTED, "constructor reverted".to_string()))
        } else if exec_status.status_code == StatusCode::Success {
            if exec_status.output_data.is_empty() {
                return Err(ActorError::unspecified("EVM constructor returned empty contract".to_string()))
            }
            // constructor ran to completion successfully and returned
            // the resulting bytecode.
            let contract_bytecode = exec_status.output_data;

            let contract_state_cid = system.flush_state()?;

            let state = State::new(
                rt.store(),
                RawBytes::new(contract_bytecode.to_vec()),
                contract_state_cid,
            )
            .map_err(|e| {
                e.downcast_default(ExitCode::USR_ILLEGAL_STATE, "failed to construct state")
            })?;
            rt.create(&state)?;

            Ok(())
        } else if let StatusCode::ActorError(e) = exec_status.status_code {
            Err(e)
        } else {
            Err(ActorError::unspecified("EVM constructor failed".to_string()))
        }
    }

    pub fn invoke_contract<BS, RT>(
        rt: &mut RT,
        method: u64,
        input_data: &RawBytes,
    ) -> Result<RawBytes, ActorError>
    where
        BS: Blockstore + Clone,
        RT: Runtime<BS>,
    {
        rt.validate_immediate_caller_accept_any()?;

        let state: State = rt.state()?;
        let bytecode: Vec<u8> = rt
            .store()
            .get(&state.bytecode)
            .map_err(|e| ActorError::unspecified(format!("failed to load bytecode: {e:?}")))?
            .ok_or_else(|| ActorError::unspecified("missing bytecode".to_string()))?;

        let bytecode = Bytecode::new(&bytecode)
            .map_err(|e| ActorError::unspecified(format!("failed to parse bytecode: {e:?}")))?;

        // clone the blockstore here to pass to the System, this is bound to the HAMT.
        let blockstore = rt.store().clone();

        // load the storage HAMT
        let mut hamt = Hamt::load(&state.contract_state, blockstore).map_err(|e| {
            ActorError::illegal_state(format!("failed to load storage HAMT on invoke: {e:?}, e"))
        })?;

        let mut system = System::new(rt, &mut hamt).map_err(|e| {
            ActorError::unspecified(format!("failed to create execution abstraction layer: {e:?}"))
        })?;

        let mut exec_state = ExecutionState::new(method, input_data.to_vec().into());

        let exec_status =
            execute(&bytecode, &mut exec_state, &mut system.reborrow()).map_err(|e| match e {
                StatusCode::ActorError(e) => e,
                _ => ActorError::unspecified(format!("EVM execution error: {e:?}")),
            })?;

        // TODO this does not return revert data yet, but it has correct semantics.
        if exec_status.reverted {
            return Err(ActorError::unchecked(EVM_CONTRACT_REVERTED, "contract reverted".to_string()))
        } else if exec_status.status_code == StatusCode::Success {
            // this needs to be outside the transaction or else rustc has a fit about
            // mutably borrowing the runtime twice.... sigh.
            let contract_state = system.flush_state()?;
            rt.transaction(|state: &mut State, _rt| {
                state.contract_state = contract_state;
                Ok(())
            })?;
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

        let output = RawBytes::from(exec_status.output_data.to_vec());
        Ok(output)
    }

    pub fn bytecode<BS, RT>(rt: &mut RT) -> Result<Cid, ActorError>
    where
        BS: Blockstore + Clone,
        RT: Runtime<BS>,
    {
        // Any caller can fetch the bytecode of a contract; this is now EXT* opcodes work.
        rt.validate_immediate_caller_accept_any()?;

        let state: State = rt.state()?;
        Ok(state.bytecode)
    }

    pub fn storage_at<BS, RT>(rt: &mut RT, params: GetStorageAtParams) -> Result<U256, ActorError>
    where
        BS: Blockstore + Clone,
        RT: Runtime<BS>,
    {
        // This method cannot be called on-chain; other on-chain logic should not be able to
        // access arbitrary storage keys from a contract.
        rt.validate_immediate_caller_is([&fvm_shared::address::Address::new_id(0)])?;

        let state: State = rt.state()?;
        let blockstore = rt.store().clone();

        // load the storage HAMT
        let mut hamt =
            Hamt::<_, _, U256>::load(&state.contract_state, blockstore).map_err(|e| {
                ActorError::illegal_state(format!(
                    "failed to load storage HAMT on invoke: {e:?}, e"
                ))
            })?;

        let mut system = System::new(rt, &mut hamt).map_err(|e| {
            ActorError::unspecified(format!("failed to create execution abstraction layer: {e:?}"))
        })?;

        system
            .get_storage(params.storage_key)
            .map_err(|st| ActorError::unspecified(format!("failed to get storage key: {}", &st)))?
            .ok_or_else(|| ActorError::not_found(String::from("storage key not found")))
    }
}

impl ActorCode for EvmContractActor {
    fn invoke_method<BS, RT>(
        rt: &mut RT,
        method: MethodNum,
        params: &RawBytes,
    ) -> Result<RawBytes, ActorError>
    where
        BS: Blockstore + Clone,
        RT: Runtime<BS>,
    {
        match FromPrimitive::from_u64(method) {
            Some(Method::Constructor) => {
                Self::constructor(rt, cbor::deserialize_params(params)?)?;
                Ok(RawBytes::default())
            }
            Some(Method::InvokeContract) => {
                Self::invoke_contract(rt, Method::InvokeContract as u64, params)
            }
            Some(Method::GetBytecode) => {
                let cid = Self::bytecode(rt)?;
                Ok(RawBytes::serialize(cid)?)
            }
            Some(Method::GetStorageAt) => {
                let value = Self::storage_at(rt, cbor::deserialize_params(params)?)?;
                Ok(RawBytes::serialize(value)?)
            }
            None => {
                Self::invoke_contract(rt, method, params)
            }
        }
    }
}

#[derive(Serialize_tuple, Deserialize_tuple)]
pub struct ConstructorParams {
    pub bytecode: RawBytes,
    pub input_data: RawBytes,
}

#[derive(Serialize_tuple, Deserialize_tuple)]
pub struct GetStorageAtParams {
    pub storage_key: U256,
}
