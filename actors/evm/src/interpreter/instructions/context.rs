use crate::interpreter::address::Address;
use {
    crate::interpreter::{ExecutionState, StatusCode, System, U256},
    fil_actors_runtime::runtime::Runtime,
    fvm_ipld_blockstore::Blockstore,
};

#[inline]
pub fn blockhash<'r, BS: Blockstore, RT: Runtime<BS>>(
    _state: &mut ExecutionState,
    _platform: &'r System<'r, BS, RT>,
) -> Result<(), StatusCode> {
    todo!("requires the client passing down the inclusion tipset hash")
}

#[inline]
pub fn caller<'r, BS: Blockstore, RT: Runtime<BS>>(
    state: &mut ExecutionState,
    platform: &'r System<'r, BS, RT>,
) {
    let id = platform.rt.message().caller().id().unwrap();
    state.stack.push(Address::from_id(id).as_evm_word())
}

#[inline]
pub fn address<'r, BS: Blockstore, RT: Runtime<BS>>(
    state: &mut ExecutionState,
    platform: &'r System<'r, BS, RT>,
) {
    let id = platform.rt.message().receiver().id().unwrap();
    state.stack.push(Address::from_id(id).as_evm_word())
}

#[inline]
pub fn origin<'r, BS: Blockstore, RT: Runtime<BS>>(
    state: &mut ExecutionState,
    platform: &'r System<'r, BS, RT>,
) {
    let id = platform.rt.message().origin().id().unwrap();
    state.stack.push(Address::from_id(id).as_evm_word())
}

#[inline]
pub fn call_value<'r, BS: Blockstore, RT: Runtime<BS>>(
    state: &mut ExecutionState,
    platform: &'r System<'r, BS, RT>,
) {
    state.stack.push(U256::from(&platform.rt.message().value_received()));
}

#[inline]
pub fn coinbase<'r, BS: Blockstore, RT: Runtime<BS>>(
    state: &mut ExecutionState,
    _platform: &'r System<'r, BS, RT>,
) {
    // TODO do we want to return the zero ID address, or just a plain 0?
    state.stack.push(U256::zero())
}

#[inline]
pub fn gas_price<'r, BS: Blockstore, RT: Runtime<BS>>(
    _state: &mut ExecutionState,
    _platform: &'r System<'r, BS, RT>,
) -> Result<(), StatusCode> {
    todo!("should return priority fee (needs syscall) + basefee")
}

#[inline]
pub fn timestamp<'r, BS: Blockstore, RT: Runtime<BS>>(
    _state: &mut ExecutionState,
    _platform: &'r System<'r, BS, RT>,
) -> Result<(), StatusCode> {
    todo!("should return the timestamp from the block header (requires syscall and FFI change)")
}

#[inline]
pub fn block_number<'r, BS: Blockstore, RT: Runtime<BS>>(
    state: &mut ExecutionState,
    platform: &'r System<'r, BS, RT>,
) {
    state.stack.push(U256::from(platform.rt.curr_epoch()))
}

#[inline]
pub fn difficulty<'r, BS: Blockstore, RT: Runtime<BS>>(
    state: &mut ExecutionState,
    _platform: &'r System<'r, BS, RT>,
) {
    state.stack.push(U256::zero())
}

#[inline]
pub fn gas_limit<'r, BS: Blockstore, RT: Runtime<BS>>(
    _state: &mut ExecutionState,
    _platform: &'r System<'r, BS, RT>,
) -> Result<(), StatusCode> {
    todo!("requires a syscall")
}

#[inline]
pub fn chain_id<'r, BS: Blockstore, RT: Runtime<BS>>(
    _state: &mut ExecutionState,
    _platform: &'r System<'r, BS, RT>,
) -> Result<(), StatusCode> {
    todo!("requires chain ID registration and configuration in the client")
}

#[inline]
pub fn base_fee<'r, BS: Blockstore, RT: Runtime<BS>>(
    state: &mut ExecutionState,
    platform: &'r System<'r, BS, RT>,
) {
    state.stack.push(U256::from(&platform.rt.base_fee()))
}
