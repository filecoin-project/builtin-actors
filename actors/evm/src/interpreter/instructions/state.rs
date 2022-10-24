use fvm_shared::address::Address;

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
    system: &'r System<'r, BS, RT>,
) -> Result<(), StatusCode> {
    let actor: EthAddress = state.stack.pop().into();

    let balance = actor
        .try_into()
        .ok()
        .and_then(|addr: Address| system.rt.resolve_address(&addr))
        .and_then(|id| system.rt.actor_balance(id).as_ref().map(U256::from))
        .unwrap_or_default();

    state.stack.push(balance);
    Ok(())
}

#[inline]
pub fn selfbalance<'r, BS: Blockstore, RT: Runtime<BS>>(
    state: &mut ExecutionState,
    system: &'r System<'r, BS, RT>,
) {
    // Returns native FIL balance of the receiver. Value precision is identical to Ethereum, so
    // no conversion needed (atto, 1e18).
    state.stack.push(U256::from(&system.rt.current_balance()))
}
