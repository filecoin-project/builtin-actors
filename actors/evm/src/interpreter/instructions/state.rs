use crate::U256;
use {
    crate::interpreter::address::EthAddress,
    crate::interpreter::{ExecutionState, StatusCode, System},
    fil_actors_runtime::runtime::Runtime,
    fvm_ipld_blockstore::Blockstore,
};

#[inline]
pub fn balance<'r, BS: Blockstore, RT: Runtime<BS>>(
    state: &mut ExecutionState,
    platform: &'r System<'r, BS, RT>,
) -> Result<(), StatusCode> {
    let actor = state.stack.pop();
    let actor_addr = EthAddress::try_from(actor)?
        .as_id_address()
        .ok_or_else(|| StatusCode::BadAddress(format!("not an actor id address: {}", actor)))?;
    state.stack.push(U256::from(&platform.rt.actor_balance(actor_addr.id().unwrap())));
    Ok(())
}

#[inline]
pub fn selfbalance<'r, BS: Blockstore, RT: Runtime<BS>>(
    state: &mut ExecutionState,
    platform: &'r System<'r, BS, RT>,
) {
    // Returns native FIL balance of the receiver. Value precision is identical to Ethereum, so
    // no conversion needed (atto, 1e18).
    state.stack.push(U256::from(&platform.rt.current_balance()))
}
