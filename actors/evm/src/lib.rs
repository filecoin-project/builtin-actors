pub mod interpreter;
mod state;

use {
    crate::interpreter::{execute, Bytecode, ExecutionState, StatusCode, System, U256},
    crate::state::State,
    bytes::Bytes,
    fil_actors_runtime::{
        actor_error, cbor,
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

#[derive(FromPrimitive)]
#[repr(u64)]
pub enum Method {
    Constructor = METHOD_CONSTRUCTOR,
    InvokeContract = 2,
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
        let mut exec_state = ExecutionState::new(Bytes::copy_from_slice(&params.input_data));

        // identify bytecode valid jump destinations
        let bytecode = Bytecode::new(&params.bytecode);

        // invoke the contract constructor
        let exec_status =
            execute(&bytecode, &mut exec_state, &mut system.reborrow()).map_err(|e| match e {
                StatusCode::ActorError(e) => e,
                _ => ActorError::unspecified(format!("EVM execution error: {e:?}")),
            })?;

        if !exec_status.reverted
            && exec_status.status_code == StatusCode::Success
            && !exec_status.output_data.is_empty()
        {
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
            Err(ActorError::unspecified(format!("EVM constructor failed: {}", exec_status.status_code)))
        }
    }

    pub fn invoke_contract<BS, RT>(
        rt: &mut RT,
        params: InvokeParams,
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

        let bytecode = Bytecode::new(&bytecode);

        // clone the blockstore here to pass to the System, this is bound to the HAMT.
        let blockstore = rt.store().clone();

        // load the storage HAMT
        let mut hamt = Hamt::load(&state.contract_state, blockstore).map_err(|e| {
            ActorError::illegal_state(format!("failed to load storage HAMT on invoke: {e:?}, e"))
        })?;

        let mut system = System::new(rt, &mut hamt).map_err(|e| {
            ActorError::unspecified(format!("failed to create execution abstraction layer: {e:?}"))
        })?;

        let mut exec_state = ExecutionState::new(Bytes::copy_from_slice(&params.input_data));

        let exec_status =
            execute(&bytecode, &mut exec_state, &mut system.reborrow()).map_err(|e| match e {
                StatusCode::ActorError(e) => e,
                _ => ActorError::unspecified(format!("EVM execution error: {e:?}")),
            })?;

        // TODO this is not the correct handling of reverts -- we need to abort (and return the
        //      output data), so that the entire transaction (including parent/sibling calls)
        //      can be reverted.
        if exec_status.status_code == StatusCode::Success {
            if !exec_status.reverted {
                // this needs to be outside the transaction or else rustc has a fit about
                // mutably borrowing the runtime twice.... sigh.
                let contract_state = system.flush_state()?;
                rt.transaction(|state: &mut State, _rt| {
                    state.contract_state = contract_state;
                    Ok(())
                })?;
            }
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
                Self::invoke_contract(rt, cbor::deserialize_params(params)?)
            }
            None => Err(actor_error!(unhandled_message; "Invalid method")),
        }
    }
}

#[derive(Serialize_tuple, Deserialize_tuple)]
pub struct ConstructorParams {
    pub bytecode: RawBytes,
    pub input_data: RawBytes,
}

#[derive(Serialize_tuple, Deserialize_tuple)]
pub struct InvokeParams {
    pub input_data: RawBytes,
}
