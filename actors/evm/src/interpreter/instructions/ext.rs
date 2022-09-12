use crate::interpreter::address::Address;
use crate::interpreter::instructions::memory::copy_to_memory;
use crate::U256;
use cid::Cid;
use fil_actors_runtime::runtime::builtins::Type;
use fil_actors_runtime::ActorError;
use fvm_shared::econ::TokenAmount;
use num_traits::Zero;
use {
    crate::interpreter::{ExecutionState, StatusCode, System},
    fil_actors_runtime::runtime::Runtime,
    fvm_ipld_blockstore::Blockstore,
};

#[inline]
pub fn extcodesize<'r, BS: Blockstore, RT: Runtime<BS>>(
    state: &mut ExecutionState,
    platform: &'r System<'r, BS, RT>,
) -> Result<(), StatusCode> {
    let addr = state.stack.pop();
    let len = get_evm_bytecode_cid(platform.rt, addr)
        .and_then(|cid| get_evm_bytecode(platform.rt, &cid))
        .map(|bytecode| bytecode.len())?;

    state.stack.push(len.into());
    Ok(())
}

pub fn extcodehash<'r, BS: Blockstore, RT: Runtime<BS>>(
    state: &mut ExecutionState,
    platform: &'r System<'r, BS, RT>,
) -> Result<(), StatusCode> {
    let addr = state.stack.pop();
    let cid = get_evm_bytecode_cid(platform.rt, addr)?;
    // Take the first 32 bytes of the Multihash
    state.stack.push(cid.hash().digest()[..32].into());
    Ok(())
}

pub fn extcodecopy<'r, BS: Blockstore, RT: Runtime<BS>>(
    state: &mut ExecutionState,
    platform: &'r System<'r, BS, RT>,
) -> Result<(), StatusCode> {
    let ExecutionState { stack, .. } = state;
    let (addr, dest_offset, data_offset, size) =
        (stack.pop(), stack.pop(), stack.pop(), stack.pop());
    let bytecode = get_evm_bytecode_cid(platform.rt, addr)
        .and_then(|cid| get_evm_bytecode(platform.rt, &cid))?;

    copy_to_memory(&mut state.memory, dest_offset, size, data_offset, bytecode.as_slice())?;

    Ok(())
}

fn get_evm_bytecode_cid<BS: Blockstore, RT: Runtime<BS>>(
    rt: &RT,
    addr: U256,
) -> Result<Cid, StatusCode> {
    let addr =
        Address::try_from(addr)?.as_id_address().expect("no support for non-ID addresses yet");

    let evm_cid = rt.get_code_cid_for_type(Type::EVM);
    let target_cid = rt.get_actor_code_cid(&addr.id().expect("not an ID address"));

    if Some(evm_cid) != target_cid {
        return Err(StatusCode::InternalError(
            "cannot invoke EXTCODESIZE for non-EVM actor".to_string(),
        )); // TODO better error code
    }

    let cid = rt
        .send(&addr, crate::Method::GetBytecode as u64, Default::default(), TokenAmount::zero())?
        .deserialize()?;
    Ok(cid)
}

pub fn get_evm_bytecode<BS: Blockstore, RT: Runtime<BS>>(
    rt: &RT,
    cid: &Cid,
) -> Result<Vec<u8>, StatusCode> {
    let raw_bytecode = rt
        .store()
        .get(cid) // TODO this is inefficient; should call stat here.
        .map_err(|e| StatusCode::InternalError(format!("failed to get bytecode block: {}", &e)))?
        .ok_or_else(|| ActorError::not_found("bytecode block not found".to_string()))?;
    Ok(raw_bytecode)
}
