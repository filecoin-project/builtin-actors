use fil_actors_runtime::ActorError;
use fil_actors_runtime::EAM_ACTOR_ADDR;
use fvm_ipld_encoding::{strict_bytes, tuple::*, RawBytes};
use fvm_shared::MethodNum;
use fvm_shared::{address::Address, econ::TokenAmount};
use serde_tuple::{Deserialize_tuple, Serialize_tuple};

use crate::interpreter::stack::Stack;
use crate::interpreter::{address::EthAddress, U256};
use crate::state::State;

use super::memory::{get_memory_region, MemoryRegion};
use {
    crate::interpreter::{ExecutionState, StatusCode, System},
    fil_actors_runtime::runtime::Runtime,
    fvm_ipld_blockstore::Blockstore,
};

pub const CREATE_METHOD_NUM: u64 = 2;
pub const CREATE2_METHOD_NUM: u64 = 3;

#[derive(Serialize_tuple, Deserialize_tuple, Clone)]
pub struct CreateParams {
    #[serde(with = "strict_bytes")]
    pub code: Vec<u8>,
    pub nonce: u64,
}

#[derive(Serialize_tuple, Deserialize_tuple, Clone)]
pub struct Create2Params {
    #[serde(with = "strict_bytes")]
    pub code: Vec<u8>,
    pub salt: [u8; 32],
}

#[derive(Serialize_tuple, Deserialize_tuple, Debug, Clone, Copy, PartialEq, Eq)]
pub struct EamReturn {
    pub actor_id: u64,
    pub robust_address: Address,
    pub eth_address: EthAddress,
}

#[inline]
pub fn create<'r, BS: Blockstore, RT: Runtime<BS>>(
    state: &mut ExecutionState,
    platform: &'r mut System<'r, BS, RT>,
) -> Result<(), StatusCode> {
    let ExecutionState { stack, memory, .. } = state;

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

    // bump nonce and flush state before send
    let nonce = platform.rt.transaction(|state: &mut State, _rt| {
        let nonce = state.nonce;
        // this may be redundant if we are compiling with checked integer math
        state.nonce = state.nonce.checked_add(1).unwrap();
        Ok(nonce)
    })?;

    let params = CreateParams { code: input_data.to_vec(), nonce };

    create_init(stack, platform, RawBytes::serialize(&params)?, CREATE_METHOD_NUM, value)
}

pub fn create2<'r, BS: Blockstore, RT: Runtime<BS>>(
    state: &mut ExecutionState,
    platform: &'r mut System<'r, BS, RT>,
) -> Result<(), StatusCode> {
    let ExecutionState { stack, memory, .. } = state;

    // see `create()` overall TODOs

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
    let params = Create2Params { code: input_data.to_vec(), salt };

    platform.rt.transaction(|state: &mut State, _rt| {
        // this may be redundant if we are compiling with checked integer math
        state.nonce = state.nonce.checked_add(1).unwrap();
        Ok(())
    })?;

    create_init(stack, platform, RawBytes::serialize(&params)?, CREATE2_METHOD_NUM, endowment)
}

/// call into Ethereum Address Manager to make the new account
fn create_init<'r, BS: Blockstore, RT: Runtime<BS>>(
    stack: &mut Stack,
    platform: &'r mut System<'r, BS, RT>,
    params: RawBytes,
    method: MethodNum,
    value: TokenAmount,
) -> Result<(), StatusCode> {
    // send bytecode & params to EAM to generate the address and contract
    let ret = platform.rt.send(&EAM_ACTOR_ADDR, method, params, value);

    // Overall future work / TODOs:
    //  readonly state things | https://github.com/filecoin-project/ref-fvm/issues/971
    //  preload state items (~eq to EVM access list) | maybe M3

    // https://github.com/ethereum/go-ethereum/blob/fb75f11e87420ec25ff72f7eeeb741fa8974e87e/core/vm/evm.go#L406-L496
    // Normally EVM will do some checks here to ensure that a contract has the capability
    // to create an actor, but here FVM does these checks for us, including:
    // - execution depth, equal to FVM's max call depth (FVM)
    // - account has enough value to send (FVM)
    // - ensuring there isn't an existing account at the generated f4 address (INIT)
    // - constructing smart contract on chain (INIT)
    // - checks if max code size is exceeded (EAM & EVM)
    // - gas cost of deployment (FVM)
    // - EIP-3541 (EVM)
    //
    // However these errors are flattened to a 0 pushed on the stack.

    // TODO revert state if error was returned (revert nonce bump)
    // https://github.com/filecoin-project/ref-fvm/issues/956

    // TODO Exit with revert if sys out of gas when subcall gas limits are introduced
    // https://github.com/filecoin-project/ref-fvm/issues/966

    let word = match ret {
        Ok(eam_ret) => {
            let ret: EamReturn = eam_ret.deserialize()?;
            ret.eth_address.as_evm_word()
        }
        Err(_) => U256::zero(),
    };

    stack.push(word);
    Ok(())
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
