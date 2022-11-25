use fvm_shared::address::Address;

use crate::U256;
use {
    crate::interpreter::address::EthAddress,
    crate::interpreter::{ExecutionState, StatusCode, System},
    fil_actors_runtime::runtime::Runtime,
};

#[inline]
pub fn balance(
    _state: &mut ExecutionState,
    system: &System<impl Runtime>,
    actor: U256,
) -> Result<U256, StatusCode> {
    let actor: EthAddress = actor.into();

    let balance = actor
        .try_into()
        .ok()
        .and_then(|addr: Address| system.rt.resolve_address(&addr))
        .and_then(|id| system.rt.actor_balance(id).as_ref().map(U256::from))
        .unwrap_or_default();

    Ok(balance)
}

#[inline]
pub fn selfbalance(state: &mut ExecutionState, system: &System<impl Runtime>) {
    // Returns native FIL balance of the receiver. Value precision is identical to Ethereum, so
    // no conversion needed (atto, 1e18).
    state.stack.push(U256::from(&system.rt.current_balance()))
}
