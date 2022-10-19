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
    system: &'r System<'r, BS, RT>,
) -> Result<(), StatusCode> {
    let bn = state.stack.pop();
    if bn.bits() > 8 {
        return Err(StatusCode::ArgumentOutOfRange(format!("invalid epoch lookback: {}", bn)));
    }
    let epoch = bn.as_u64() as i64;
    if let Some(cid) = system.rt.tipset_cid(epoch) {
        let mut hash = cid.hash().digest();
        if hash.len() > 32 {
            hash = &hash[..32]
        }
        state.stack.push(U256::from_big_endian(hash));
        Ok(())
    } else {
        Err(StatusCode::InvalidArgument(format!("no tipset for epoch lookback at: {}", epoch)))
    }
}

#[inline]
pub fn caller<'r, BS: Blockstore, RT: Runtime<BS>>(
    state: &mut ExecutionState,
    _: &'r System<'r, BS, RT>,
) {
    state.stack.push(state.caller.as_evm_word())
}

#[inline]
pub fn address<'r, BS: Blockstore, RT: Runtime<BS>>(
    state: &mut ExecutionState,
    system: &'r System<'r, BS, RT>,
) {
    let id = system.rt.message().receiver().id().unwrap();
    state.stack.push(EthAddress::from_id(id).as_evm_word())
}

#[inline]
pub fn origin<'r, BS: Blockstore, RT: Runtime<BS>>(
    state: &mut ExecutionState,
    system: &'r System<'r, BS, RT>,
) {
    let id = system.rt.message().origin().id().unwrap();
    state.stack.push(EthAddress::from_id(id).as_evm_word())
}

#[inline]
pub fn call_value<'r, BS: Blockstore, RT: Runtime<BS>>(
    state: &mut ExecutionState,
    system: &'r System<'r, BS, RT>,
) {
    state.stack.push(U256::from(&system.rt.message().value_received()));
}

#[inline]
pub fn coinbase<'r, BS: Blockstore, RT: Runtime<BS>>(
    state: &mut ExecutionState,
    _system: &'r System<'r, BS, RT>,
) {
    // TODO do we want to return the zero ID address, or just a plain 0?
    state.stack.push(U256::zero())
}

#[inline]
pub fn gas_price<'r, BS: Blockstore, RT: Runtime<BS>>(
    state: &mut ExecutionState,
    system: &'r System<'r, BS, RT>,
) {
    let effective_price = system.rt.base_fee() + system.rt.message().gas_premium();
    state.stack.push(U256::from(&effective_price));
}

#[inline]
pub fn gas<'r, BS: Blockstore, RT: Runtime<BS>>(
    state: &mut ExecutionState,
    system: &'r System<'r, BS, RT>,
) {
    state.stack.push(U256::from(system.rt.gas_available()));
}

#[inline]
pub fn timestamp<'r, BS: Blockstore, RT: Runtime<BS>>(
    state: &mut ExecutionState,
    system: &'r System<'r, BS, RT>,
) {
    state.stack.push(U256::from(system.rt.tipset_timestamp()));
}

#[inline]
pub fn block_number<'r, BS: Blockstore, RT: Runtime<BS>>(
    state: &mut ExecutionState,
    system: &'r System<'r, BS, RT>,
) {
    state.stack.push(U256::from(system.rt.curr_epoch()));
}

#[inline]
pub fn difficulty<'r, BS: Blockstore, RT: Runtime<BS>>(
    state: &mut ExecutionState,
    _system: &'r System<'r, BS, RT>,
) {
    state.stack.push(U256::zero());
}

#[inline]
pub fn gas_limit<'r, BS: Blockstore, RT: Runtime<BS>>(
    state: &mut ExecutionState,
    _system: &'r System<'r, BS, RT>,
) {
    const BLOCK_GAS_LIMIT: u64 = 10_000_000_000u64;
    state.stack.push(U256::from(BLOCK_GAS_LIMIT));
}

#[inline]
pub fn chain_id<'r, BS: Blockstore, RT: Runtime<BS>>(
    state: &mut ExecutionState,
    _system: &'r System<'r, BS, RT>,
) {
    state.stack.push(U256::from(chainid::CHAINID));
}

#[inline]
pub fn base_fee<'r, BS: Blockstore, RT: Runtime<BS>>(
    state: &mut ExecutionState,
    system: &'r System<'r, BS, RT>,
) {
    state.stack.push(U256::from(&system.rt.base_fee()))
}
