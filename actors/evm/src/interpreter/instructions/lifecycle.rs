use bytes::Bytes;

use fil_actors_runtime::deserialize_block;
use fil_actors_runtime::EAM_ACTOR_ADDR;
use fvm_ipld_encoding::ipld_block::IpldBlock;
use fvm_ipld_encoding::{strict_bytes, tuple::*};
use fvm_shared::sys::SendFlags;
use fvm_shared::MethodNum;
use fvm_shared::METHOD_SEND;
use fvm_shared::{address::Address, econ::TokenAmount};
use serde_tuple::{Deserialize_tuple, Serialize_tuple};

use crate::interpreter::Output;
use crate::interpreter::{address::EthAddress, U256};

use super::memory::{get_memory_region, MemoryRegion};
use {
    crate::interpreter::{ExecutionState, StatusCode, System},
    fil_actors_runtime::runtime::Runtime,
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
    #[serde(with = "strict_bytes")]
    pub salt: [u8; 32],
}

#[derive(Serialize_tuple, Deserialize_tuple, Debug, Clone, Copy, PartialEq, Eq)]
pub struct EamReturn {
    pub actor_id: u64,
    pub robust_address: Option<Address>,
    pub eth_address: EthAddress,
}

#[inline]
pub fn create(
    state: &mut ExecutionState,
    system: &mut System<impl Runtime>,
    value: U256,
    offset: U256,
    size: U256,
) -> Result<U256, StatusCode> {
    if system.readonly {
        return Err(StatusCode::StaticModeViolation);
    }

    let ExecutionState { stack: _, memory, .. } = state;

    let value = TokenAmount::from(&value);
    if value > system.rt.current_balance() {
        return Ok(U256::zero());
    }
    let input_region =
        get_memory_region(memory, offset, size).map_err(|_| StatusCode::InvalidMemoryAccess)?;

    let input_data = if let Some(MemoryRegion { offset, size }) = input_region {
        &memory[offset..][..size.get()]
    } else {
        &[]
    };

    let nonce = system.increment_nonce();
    let params = CreateParams { code: input_data.to_vec(), nonce };
    create_init(system, IpldBlock::serialize_cbor(&params)?, CREATE_METHOD_NUM, value)
}

pub fn create2(
    state: &mut ExecutionState,
    system: &mut System<impl Runtime>,
    endowment: U256,
    offset: U256,
    size: U256,
    salt: U256,
) -> Result<U256, StatusCode> {
    if system.readonly {
        return Err(StatusCode::StaticModeViolation);
    }

    let ExecutionState { stack: _, memory, .. } = state;

    // see `create()` overall TODOs
    let endowment = TokenAmount::from(&endowment);
    if endowment > system.rt.current_balance() {
        return Ok(U256::zero());
    }

    let input_region =
        get_memory_region(memory, offset, size).map_err(|_| StatusCode::InvalidMemoryAccess)?;

    // BE encoded array
    let salt: [u8; 32] = salt.into();

    let input_data = if let Some(MemoryRegion { offset, size }) = input_region {
        &memory[offset..][..size.get()]
    } else {
        &[]
    };
    let params = Create2Params { code: input_data.to_vec(), salt };

    system.increment_nonce();
    create_init(system, IpldBlock::serialize_cbor(&params)?, CREATE2_METHOD_NUM, endowment)
}

/// call into Ethereum Address Manager to make the new account
#[inline]
fn create_init(
    system: &mut System<impl Runtime>,
    params: Option<IpldBlock>,
    method: MethodNum,
    value: TokenAmount,
) -> Result<U256, StatusCode> {
    // send bytecode & params to EAM to generate the address and contract
    let ret = system.send(&EAM_ACTOR_ADDR, method, params, value, None, SendFlags::default());

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

    Ok(match ret {
        Ok(eam_ret) => {
            let ret: EamReturn = deserialize_block(eam_ret)?;
            ret.eth_address.as_evm_word()
        }
        Err(_) => U256::zero(),
    })
}

#[inline]
pub fn selfdestruct(
    _state: &mut ExecutionState,
    system: &mut System<impl Runtime>,
    beneficiary: U256,
) -> Result<Output, StatusCode> {
    use crate::interpreter::output::Outcome;

    if system.readonly {
        return Err(StatusCode::StaticModeViolation);
    }

    // Try to give funds to the beneficiary. If this fails, we just keep them.
    let beneficiary: EthAddress = beneficiary.into();
    let beneficiary: Address = beneficiary.into();
    let balance = system.rt.current_balance();
    let _ = system.rt.send(&beneficiary, METHOD_SEND, None, balance);

    // Now mark ourselves as deleted.
    system.mark_selfdestructed();

    // And "return".
    //
    // 1. In the constructor, this will set our code to "empty". This is correct.
    // 2. Otherwise, we'll successfully return nothing to the caller.
    Ok(Output { outcome: Outcome::Return, return_data: Bytes::new() })
}
