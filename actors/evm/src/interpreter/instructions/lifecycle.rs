use fil_actors_runtime::EAM_ACTOR_ADDR;
use fil_actors_runtime::ActorError;
use fvm_ipld_encoding::{
    strict_bytes,
    tuple::*,
    RawBytes,
};
use fvm_shared::{
    address::Address,
    econ::TokenAmount,
};
use serde_tuple::{Deserialize_tuple, Serialize_tuple};

use crate::interpreter::{address::EthAddress, U256};

use super::memory::{get_memory_region, MemoryRegion};
use {
    crate::interpreter::{ExecutionState, StatusCode, System},
    fil_actors_runtime::runtime::Runtime,
    fvm_ipld_blockstore::Blockstore,
};

#[derive(Serialize_tuple, Deserialize_tuple)]
pub struct EamReturn {
    pub actor_id: u64,
    pub robust_address: Address,
    pub eth_address: EthAddress,
}

#[inline]
pub fn create<'r, BS: Blockstore, RT: Runtime<BS>>(
    state: &mut ExecutionState,
    platform: &'r mut System<'r, BS, RT>,
    create2: bool,
) -> Result<U256, StatusCode> {
    const CREATE_METHOD_NUM: u64 = 2;
    const CREATE2_METHOD_NUM: u64 = 3;

    let ExecutionState { stack, memory, .. } = state;
    // TODO: readonly state things

    // create2
    let ret: Result<RawBytes, ActorError> = if create2 {
        #[derive(Serialize_tuple, Deserialize_tuple)]
        struct Create2Params {
            #[serde(with = "strict_bytes")]
            code: Vec<u8>,
            salt: [u8; 32],
        }

        let endowment = stack.pop();
        let (offset, size) = (stack.pop(), stack.pop());
        let salt = stack.pop();

        let endowment = TokenAmount::from(&endowment);
        let input_region =
            get_memory_region(memory, offset, size).map_err(|_| StatusCode::InvalidMemoryAccess)?;

        // BE encoded array
        let salt: [u8; 32] = salt.into();

        let input_data = if let Some(MemoryRegion { offset, size }) = input_region {
            &memory[offset..][..size.get()]
        } else {
            return Err(StatusCode::ActorError(ActorError::illegal_argument(
                "initcode not in memory range".to_string(),
            )));
        };

        // call into Ethereum Address Manager to make the new account
        let params = Create2Params { code: input_data.to_vec(), salt };

        platform.rt.send(
            &EAM_ACTOR_ADDR,
            CREATE2_METHOD_NUM,
            RawBytes::serialize(&params)?,
            endowment,
        )
        // errs
    } else {
        // create1
        #[derive(Serialize_tuple, Deserialize_tuple)]
        struct CreateParams {
            #[serde(with = "strict_bytes")]
            code: Vec<u8>,
            nonce: u64,
        }

        let value = stack.pop();
        let (offset, size) = (stack.pop(), stack.pop());

        let value = TokenAmount::from(&value);
        let input_region =
            get_memory_region(memory, offset, size).map_err(|_| StatusCode::InvalidMemoryAccess)?;

        let input_data = if let Some(MemoryRegion { offset, size }) = input_region {
            &memory[offset..][..size.get()]
        } else {
            return Err(StatusCode::ActorError(ActorError::assertion_failed(
                "inicode not in memory range".to_string(),
            )));
        };

        // call into Ethereum Address Manager to make the new account
        let params = CreateParams { code: input_data.to_vec(), nonce: state.nonce };
        
        platform.rt.send(&EAM_ACTOR_ADDR, CREATE_METHOD_NUM, RawBytes::serialize(&params)?, value)
    };

    // bump nonce
    state.nonce += 1;
    // flush nonce change 
    platform.flush_state().unwrap();
    
    // TODO handle nonce change revert 

    let word = match ret {
        Ok(eam_ret) => {
            let ret: EamReturn = eam_ret.deserialize()?;
            ret.eth_address.as_evm_word()
        }
        Err(_) => U256::zero(),
    };

    stack.push(word);

    todo!()
}

#[inline]
pub fn selfdestruct<'r, BS: Blockstore, RT: Runtime<BS>>(
    state: &mut ExecutionState,
    _system: &'r mut System<'r, BS, RT>,
) -> Result<(), StatusCode> {
    let beneficiary_addr = EthAddress::try_from(state.stack.pop())?;
    let id_addr = beneficiary_addr.as_id_address().expect("no support for non-ID addresses yet");
    state.selfdestroyed = Some(id_addr);
    Ok(())
}
