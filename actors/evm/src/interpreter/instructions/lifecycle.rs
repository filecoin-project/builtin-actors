use fil_actors_runtime::builtin::singletons::EAM_ACTOR_ID;
use fil_actors_runtime::EAM_ACTOR_ADDR;
use fil_actors_runtime::{actor_error, runtime::builtins::Type, ActorError};
use fvm_ipld_encoding::{
    serde_bytes::{self, Deserialize},
    tuple::*,
    RawBytes,
};
use fvm_shared::{
    address::Address,
    bigint::{self, BigUint},
    econ::TokenAmount,
};
use serde::Deserializer;
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
    #[serde(with = "serde_bytes")]
    pub eth_address: EthAddress,
}

#[inline]
pub fn create<'r, BS: Blockstore, RT: Runtime<BS>>(
    state: &mut ExecutionState,
    platform: &'r System<'r, BS, RT>,
    create2: bool,
) -> Result<U256, StatusCode> {

    // TODO be more careful with errors
    const CREATE_METHOD_NUM: u64 = 2;
    const CREATE2_METHOD_NUM: u64 = 3;

    let ExecutionState { stack, memory, .. } = state;
    // readonly things?

    // create2
    let ret: Result<RawBytes, ActorError> = if create2 {
        #[derive(Serialize_tuple, Deserialize_tuple)]
        struct Create2Params {
            #[serde(with = "serde_bytes")]
            code: Vec<u8>,
            salt: [u8; 32],
        }

        let endowment = stack.pop();
        let (offset, size) = (stack.pop(), stack.pop());
        let salt = stack.pop();

        let endowment = TokenAmount::from(&endowment);
        let input_region =
            get_memory_region(memory, offset, size).map_err(|_| StatusCode::InvalidMemoryAccess)?;

        let stackvalue = size; // ?

        let salt = {
            let mut buf = [0u8; 32];
            // TODO make sure this is the right encoding
            salt.to_little_endian(&mut buf);
            buf
        };

        let input_data = if let Some(MemoryRegion { offset, size }) = input_region {
            &memory[offset..][..size.get()]
        } else {
            return Err(StatusCode::ActorError(ActorError::assertion_failed(
                "inicode not in memory range".to_string(),
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
    } else { // create1
        #[derive(Serialize_tuple, Deserialize_tuple)]
        struct CreateParams {
            #[serde(with = "serde_bytes")]
            code: Vec<u8>,
            nonce: u64,
        }

        let value = stack.pop();
        let (offset, size) = (stack.pop(), stack.pop());
        let input = stack.pop();

        let input_region =
            get_memory_region(memory, offset, size).map_err(|_| StatusCode::InvalidMemoryAccess)?;
        let value = TokenAmount::from(&value);

        let input_data = if let Some(MemoryRegion { offset, size }) = input_region {
            &memory[offset..][..size.get()]
        } else {
            return Err(StatusCode::ActorError(ActorError::assertion_failed(
                "inicode not in memory range".to_string(),
            )));
        };

        let params = CreateParams { code: input_data.to_vec(), nonce: state.nonce };

        // bump nonce
        state.nonce += 1; 

        // TODO access control list?

        platform.rt.send(
            &EAM_ACTOR_ADDR,
            CREATE_METHOD_NUM,
            RawBytes::serialize(&params)?,
            value,
        )
    };
    
    

    let word = match ret {
        Ok(eam_ret) => {
            let ret: EamReturn = eam_ret.deserialize()?;
            ret.eth_address.as_evm_word()
        },
        Err(_) => U256::zero(),
    };

    stack.push(word);

    todo!()
}

struct Create2Ret {
    out: Vec<u8>,
    // f4 address
    addr: Address,
    // todo gas num type
    leftover_gas: i64,
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
