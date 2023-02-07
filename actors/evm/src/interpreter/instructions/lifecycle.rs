use fil_actors_evm_shared::address::EthAddress;
use fil_actors_evm_shared::uints::U256;
use fil_actors_runtime::ActorError;
use fil_actors_runtime::EAM_ACTOR_ADDR;
use fil_actors_runtime::{deserialize_block, extract_send_result};
use fvm_ipld_encoding::ipld_block::IpldBlock;
use fvm_shared::sys::SendFlags;
use fvm_shared::MethodNum;
use fvm_shared::METHOD_SEND;
use fvm_shared::{address::Address, econ::TokenAmount};

use crate::ext::eam;
use crate::interpreter::Output;
use crate::EVM_CONTRACT_SELFDESTRUCT_FAILED;

use super::memory::{get_memory_region, MemoryRegion};
use {
    crate::interpreter::{ExecutionState, System},
    fil_actors_runtime::runtime::Runtime,
};

#[inline]
pub fn create(
    state: &mut ExecutionState,
    system: &mut System<impl Runtime>,
    value: U256,
    offset: U256,
    size: U256,
) -> Result<U256, ActorError> {
    if system.readonly {
        return Err(ActorError::read_only("create called while read-only".into()));
    }

    let ExecutionState { stack: _, memory, .. } = state;

    let value = TokenAmount::from(&value);
    if value > system.rt.current_balance() {
        return Ok(U256::zero());
    }
    let input_region = get_memory_region(memory, offset, size)?;

    let input_data = if let Some(MemoryRegion { offset, size }) = input_region {
        &memory[offset..][..size.get()]
    } else {
        &[]
    };

    // We increment the nonce earlier than in the EVM. See the comment in `create2` for details.
    let nonce = system.increment_nonce();
    let params = eam::CreateParams { code: input_data.to_vec(), nonce };
    create_init(system, IpldBlock::serialize_cbor(&params)?, eam::CREATE_METHOD_NUM, value)
}

pub fn create2(
    state: &mut ExecutionState,
    system: &mut System<impl Runtime>,
    endowment: U256,
    offset: U256,
    size: U256,
    salt: U256,
) -> Result<U256, ActorError> {
    if system.readonly {
        return Err(ActorError::read_only("create2 called while read-only".into()));
    }

    let ExecutionState { stack: _, memory, .. } = state;

    let endowment = TokenAmount::from(&endowment);
    if endowment > system.rt.current_balance() {
        return Ok(U256::zero());
    }

    let input_region = get_memory_region(memory, offset, size)?;

    // BE encoded array
    let salt: [u8; 32] = salt.into();

    let input_data = if let Some(MemoryRegion { offset, size }) = input_region {
        &memory[offset..][..size.get()]
    } else {
        &[]
    };
    let params = eam::Create2Params { code: input_data.to_vec(), salt };

    // We increment the nonce earlier than in the EVM, but this is unlikely to cause issues:
    //
    // 1. Like the EVM, we increment the nonce on address conflict (effectively "skipping" the
    //    address).
    // 2. Like the EVM, we increment the nonce even if the target contract runs out of gas.
    // 4. Like the EVM, we don't increment the nonce if the caller doesn't have enough funds to
    //    cover the endowment.
    // 4. Unlike the EVM, we increment the nonce if contract creation fails because we're at the max
    //    stack depth. However, given that there are other ways to increment the nonce without
    //    deploying a contract (e.g., 2), this shouldn't be an issue.
    system.increment_nonce();
    create_init(system, IpldBlock::serialize_cbor(&params)?, eam::CREATE2_METHOD_NUM, endowment)
}

/// call into Ethereum Address Manager to make the new account
#[inline]
fn create_init(
    system: &mut System<impl Runtime>,
    params: Option<IpldBlock>,
    method: MethodNum,
    value: TokenAmount,
) -> Result<U256, ActorError> {
    // Apply EIP-150
    let gas_limit = (63 * system.rt.gas_available()) / 64;

    // send bytecode & params to EAM to generate the address and contract
    let ret =
        system.send(&EAM_ACTOR_ADDR, method, params, value, Some(gas_limit), SendFlags::default());

    Ok(match ret {
        Ok(eam_ret) => {
            let ret: eam::CreateReturn = deserialize_block(eam_ret)?;
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
) -> Result<Output, ActorError> {
    use crate::interpreter::output::Outcome;

    if system.readonly {
        return Err(ActorError::read_only("selfdestruct called while read-only".into()));
    }

    // Try to give funds to the beneficiary. If this fails, we abort the entire call. This can only
    // fail if:
    //
    // 1. The target address is an embedded ID address and said actor doesn't exist.
    // 2. We're at the maximum call depth.
    // 3. This call would cause us to exceed some system limit (e.g., a memory limit).
    let beneficiary: EthAddress = beneficiary.into();
    let beneficiary: Address = beneficiary.into();
    let balance = system.rt.current_balance();
    extract_send_result(system.rt.send_simple(&beneficiary, METHOD_SEND, None, balance)).map_err(
        |e| {
            ActorError::unchecked(
                EVM_CONTRACT_SELFDESTRUCT_FAILED,
                format!(
                    "failed to transfer funds to beneficiary {beneficiary} on SELFDESTRUCT: {e}"
                ),
            )
        },
    )?;

    // Now mark ourselves as deleted.
    system.mark_selfdestructed();

    // And "return".
    //
    // 1. In the constructor, this will set our code to "empty". This is correct.
    // 2. Otherwise, we'll successfully return nothing to the caller.
    Ok(Output { outcome: Outcome::Return, return_data: Vec::new() })
}
