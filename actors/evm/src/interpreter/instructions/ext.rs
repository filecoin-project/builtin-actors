use crate::interpreter::address::Address;
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
    let addr = Address::try_from(state.stack.pop())?
        .as_id_address()
        .expect("no support for non-ID addresses yet");

    let System { rt, .. } = platform;
    let evm_cid = rt.get_code_cid_for_type(Type::EVM);
    let target_cid = rt.get_actor_code_cid(&addr.id().expect("not an ID address"));

    if Some(evm_cid) != target_cid {
        return Err(StatusCode::InternalError(
            "cannot invoke EXTCODESIZE for non-EVM actor".to_string(),
        )); // TODO better error code
    }

    let bytecode_cid: Cid = rt
        .send(&addr, crate::Method::GetBytecode as u64, Default::default(), TokenAmount::zero())?
        .deserialize()?;

    let raw_bytecode = rt
        .store()
        .get(&bytecode_cid) // TODO this is inefficient; should call stat here.
        .map_err(|e| StatusCode::InternalError(format!("failed to get bytecode block: {}", &e)))?
        .ok_or_else(|| ActorError::not_found("bytecode block not found".to_string()))?;

    state.stack.push(raw_bytecode.len().into());
    Ok(())
}

pub fn extcodehash<'r, BS: Blockstore, RT: Runtime<BS>>(
    _state: &mut ExecutionState,
    _platform: &'r System<'r, BS, RT>,
) -> Result<(), StatusCode> {
    // TODO

    todo!();
}

pub fn extcodecopy<'r, BS: Blockstore, RT: Runtime<BS>>(
    _state: &mut ExecutionState,
    _platform: &'r System<'r, BS, RT>,
) -> Result<(), StatusCode> {
    todo!();
}
