mod interpreter;
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
fil_actors_runtime::wasm_trampoline!(EvmRuntimeActor);

/// Maximum allowed EVM bytecode size.
/// The contract code size limit is 24kB.
const MAX_CODE_SIZE: usize = 24 << 10;

#[derive(FromPrimitive)]
#[repr(u64)]
pub enum Method {
    Constructor = METHOD_CONSTRUCTOR,
    InvokeContract = 2,
}

pub struct EvmRuntimeActor;
impl EvmRuntimeActor {
    pub fn constructor<BS, RT>(rt: &mut RT, params: ConstructorParams) -> Result<(), ActorError>
    where
        BS: Blockstore,
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

        // initialize contract state
        let init_contract_state_cid =
            Hamt::<_, U256, U256>::new(rt.store()).flush().map_err(|e| {
                ActorError::unspecified(format!("failed to flush contract state: {e:?}"))
            })?;
        // create an instance of the platform abstraction layer -- note: do we even need this?
        let system = System::new(rt, init_contract_state_cid).map_err(|e| {
            ActorError::unspecified(format!("failed to create execution abstraction layer: {e:?}"))
        })?;
        // create a new execution context
        let mut exec_state = ExecutionState::new(Bytes::copy_from_slice(&params.input_data));
        // identify bytecode valid jump destinations
        let bytecode = Bytecode::new(&params.bytecode)
            .map_err(|e| ActorError::unspecified(format!("failed to parse bytecode: {e:?}")))?;
        // invoke the contract constructor
        let exec_status = execute(&bytecode, &mut exec_state, &system)
            .map_err(|e| ActorError::unspecified(format!("EVM execution error: {e:?}")))?;

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
        } else {
            Err(ActorError::unspecified("EVM constructor failed".to_string()))
        }
    }

    pub fn invoke_contract<BS, RT>(rt: &mut RT, _params: &RawBytes) -> Result<RawBytes, ActorError>
    where
        BS: Blockstore,
        RT: Runtime<BS>,
    {
        rt.validate_immediate_caller_accept_any()?;
        // let state: ContractState = rt.state()?;
        // let message = Message {
        //   kind: fvm_evm::CallKind::Call,
        //   is_static: false,
        //   depth: 1,
        //   gas: 2100,
        //   recipient: H160::zero(),
        //   sender: H160::zero(),
        //   input_data: Bytes::new(),
        //   value: U256::zero(),
        // };

        // let bytecode: Vec<_> = from_slice(&ipld::get(&state.bytecode).map_err(|e| {
        //   ActorError::illegal_state(format!("failed to load bytecode: {e:?}"))
        // })?)
        // .map_err(|e| ActorError::unspecified(format!("failed to load bytecode:
        // {e:?}")))?;

        // // EVM contract bytecode
        // let bytecode = Bytecode::new(&bytecode)
        //   .map_err(|e| ActorError::unspecified(format!("invalid bytecode: {e:?}")))?;

        // // the execution state of the EVM, stack, heap, etc.
        // let mut runtime = ExecutionState::new(&message);

        // // the interface between the EVM interpretter and the FVM system
        // let mut system = System::new(state.state, rt, state.bridge,
        // state.self_address)   .map_err(|e|
        // ActorError::unspecified(format!("failed to create runtime: {e:?}")))?;

        // // invoke the bytecode using the current state and the platform interface
        // let output = execute(&bytecode, &mut runtime, &mut system)
        //   .map_err(|e| ActorError::unspecified(format!("contract execution error:
        // {e:?}")))?;

        // log(format!("evm output: {output:?}"));
        Ok(RawBytes::default())
    }
}

impl ActorCode for EvmRuntimeActor {
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
            Some(Method::InvokeContract) => Self::invoke_contract(rt, params),
            None => Err(actor_error!(unhandled_message; "Invalid method")),
        }
    }
}

#[derive(Serialize_tuple, Deserialize_tuple)]
pub struct ConstructorParams {
    pub bytecode: RawBytes,
    pub input_data: RawBytes,
}
