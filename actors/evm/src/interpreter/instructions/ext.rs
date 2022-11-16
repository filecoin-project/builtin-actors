use crate::interpreter::address::EthAddress;
use crate::interpreter::instructions::memory::copy_to_memory;
use crate::U256;
use cid::Cid;
use fil_actors_runtime::runtime::builtins::Type;
use fil_actors_runtime::ActorError;
use fvm_ipld_blockstore::Blockstore;
use fvm_shared::{address::Address, econ::TokenAmount};
use num_traits::Zero;
use {
    crate::interpreter::{ExecutionState, StatusCode, System},
    fil_actors_runtime::runtime::Runtime,
};

#[inline]
pub fn extcodesize(
    state: &mut ExecutionState,
    system: &System<impl Runtime>,
) -> Result<(), StatusCode> {
    let addr = state.stack.pop();
    // TODO we're fetching the entire block here just to get its size. We should instead use
    //  the ipld::block_stat syscall, but the Runtime nor the Blockstore expose it.
    //  Tracked in https://github.com/filecoin-project/ref-fvm/issues/867
    let len = get_evm_bytecode_cid(system.rt, addr)
        .and_then(|cid| get_evm_bytecode(system.rt, &cid))
        .map(|bytecode| bytecode.len())?;

    state.stack.push(len.into());
    Ok(())
}

pub fn extcodehash(
    state: &mut ExecutionState,
    system: &System<impl Runtime>,
) -> Result<(), StatusCode> {
    let addr = state.stack.pop();
    let cid = get_evm_bytecode_cid(system.rt, addr)?;
    let digest = cid.hash().digest();
    // Take the first 32 bytes of the Multihash
    let digest_len = digest.len().min(32);
    state.stack.push(digest[..digest_len].into());
    Ok(())
}

pub fn extcodecopy(
    state: &mut ExecutionState,
    system: &System<impl Runtime>,
) -> Result<(), StatusCode> {
    let ExecutionState { stack, .. } = state;
    let (addr, dest_offset, data_offset, size) =
        (stack.pop(), stack.pop(), stack.pop(), stack.pop());
    let bytecode =
        get_evm_bytecode_cid(system.rt, addr).and_then(|cid| get_evm_bytecode(system.rt, &cid))?;

    copy_to_memory(&mut state.memory, dest_offset, size, data_offset, bytecode.as_slice(), true)
}

pub fn get_evm_bytecode_cid(rt: &impl Runtime, addr: U256) -> Result<Cid, StatusCode> {
    let addr: EthAddress = addr.into();
    let addr: Address = addr.try_into()?;
    // TODO: just return none in most of these cases?
    let actor_id = rt.resolve_address(&addr).ok_or_else(|| {
        StatusCode::InvalidArgument("failed to resolve address".to_string())
        // TODO better error code
    })?;

    let evm_cid = rt.get_code_cid_for_type(Type::EVM);
    let target_cid = rt.get_actor_code_cid(&actor_id);

    if Some(evm_cid) != target_cid {
        return Err(StatusCode::InvalidArgument(
            "cannot invoke EXTCODESIZE for non-EVM actor".to_string(),
        )); // TODO better error code
    }

    let cid = rt
        .send(&addr, crate::Method::GetBytecode as u64, Default::default(), TokenAmount::zero())?
        .deserialize()?;
    Ok(cid)
}

pub fn get_evm_bytecode(rt: &impl Runtime, cid: &Cid) -> Result<Vec<u8>, StatusCode> {
    let raw_bytecode = rt
        .store()
        .get(cid) // TODO this is inefficient; should call stat here.
        .map_err(|e| StatusCode::InternalError(format!("failed to get bytecode block: {}", &e)))?
        .ok_or_else(|| ActorError::not_found("bytecode block not found".to_string()))?;
    Ok(raw_bytecode)
}
