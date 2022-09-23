use {
    crate::interpreter::address::EthAddress,
    crate::interpreter::{ExecutionState, StatusCode, System, U256},
    fil_actors_runtime::runtime::chainid,
    fil_actors_runtime::runtime::Runtime,
    fvm_ipld_blockstore::Blockstore,
};

#[inline]
pub fn blockhash<'r, BS: Blockstore, RT: Runtime<BS>>(
    state: &mut ExecutionState,
    platform: &'r System<'r, BS, RT>,
) -> Result<(), StatusCode> {
    let bn = state.stack.pop();
    let bn8 = u8::try_from(bn)
        .map_err(|_| StatusCode::ArgumentOutOfRange("expected byte".to_string()))?;
    state.stack.push(U256::from_big_endian(&platform.rt.environment().blockhash(bn8)));
    Ok(())
}

#[inline]
pub fn caller<'r, BS: Blockstore, RT: Runtime<BS>>(
    state: &mut ExecutionState,
    platform: &'r System<'r, BS, RT>,
) {
    let id = platform.rt.message().caller().id().unwrap();
    state.stack.push(EthAddress::from_id(id).as_evm_word())
}

#[inline]
pub fn address<'r, BS: Blockstore, RT: Runtime<BS>>(
    state: &mut ExecutionState,
    platform: &'r System<'r, BS, RT>,
) {
    let id = platform.rt.message().receiver().id().unwrap();
    state.stack.push(EthAddress::from_id(id).as_evm_word())
}

#[inline]
pub fn origin<'r, BS: Blockstore, RT: Runtime<BS>>(
    state: &mut ExecutionState,
    platform: &'r System<'r, BS, RT>,
) {
    let id = platform.rt.message().origin().id().unwrap();
    state.stack.push(EthAddress::from_id(id).as_evm_word())
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
    state: &mut ExecutionState,
    platform: &'r System<'r, BS, RT>,
) {
    state.stack.push(U256::from(platform.rt.environment().gas_price()));
}

#[inline]
pub fn gas<'r, BS: Blockstore, RT: Runtime<BS>>(
    state: &mut ExecutionState,
    platform: &'r System<'r, BS, RT>,
) {
    state.stack.push(U256::from(platform.rt.gas_available()));
}

#[inline]
pub fn timestamp<'r, BS: Blockstore, RT: Runtime<BS>>(
    state: &mut ExecutionState,
    platform: &'r System<'r, BS, RT>,
) {
    state.stack.push(U256::from(platform.rt.environment().timestamp()));
}

#[inline]
pub fn block_number<'r, BS: Blockstore, RT: Runtime<BS>>(
    state: &mut ExecutionState,
    platform: &'r System<'r, BS, RT>,
) {
    state.stack.push(U256::from(platform.rt.curr_epoch()));
}

#[inline]
pub fn difficulty<'r, BS: Blockstore, RT: Runtime<BS>>(
    state: &mut ExecutionState,
    _platform: &'r System<'r, BS, RT>,
) {
    state.stack.push(U256::zero());
}

#[inline]
pub fn gas_limit<'r, BS: Blockstore, RT: Runtime<BS>>(
    state: &mut ExecutionState,
    platform: &'r System<'r, BS, RT>,
) {
    state.stack.push(U256::from(platform.rt.environment().gas_limit()));
}

#[inline]
pub fn chain_id<'r, BS: Blockstore, RT: Runtime<BS>>(
    state: &mut ExecutionState,
    _platform: &'r System<'r, BS, RT>,
) {
    state.stack.push(U256::from(chainid::CHAINID));
}

#[inline]
pub fn base_fee<'r, BS: Blockstore, RT: Runtime<BS>>(
    state: &mut ExecutionState,
    platform: &'r System<'r, BS, RT>,
) {
    state.stack.push(U256::from(&platform.rt.base_fee()))
}
