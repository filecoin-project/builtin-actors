use fvm_shared::clock::ChainEpoch;

use {
    crate::interpreter::{ExecutionState, System, U256, StatusCode},
    fil_actors_runtime::runtime::chainid,
    fil_actors_runtime::runtime::Runtime,
};

#[inline]
pub fn blockhash(state: &mut ExecutionState, system: &System<impl Runtime>) {
    let bn = state.stack.pop();
    let result = bn
        .try_into()
        .ok()
        .filter(|&height: &ChainEpoch| {
            // The EVM allows fetching blockhashes from the 256 _previous_ blocks.
            // TODO: we can consider extending this to allow the full range.
            // Also relates to https://github.com/filecoin-project/ref-fvm/issues/1023 (we might
            // want to keep some of these restrictions).
            let curr_epoch = system.rt.curr_epoch();
            height >= curr_epoch - 256 && height < curr_epoch
        })
        .and_then(|height| system.rt.tipset_cid(height))
        .map(|cid| {
            let mut hash = cid.hash().digest();
            if hash.len() > 32 {
                hash = &hash[..32]
            }
            U256::from_big_endian(hash)
        })
        .unwrap_or_default();
    state.stack.push(result);
}

#[inline]
pub fn caller(state: &mut ExecutionState, _: &System<impl Runtime>)  -> Result<U256, StatusCode>  {
    Ok(state.caller.as_evm_word())
}

#[inline]
pub fn address(state: &mut ExecutionState, _system: &System<impl Runtime>) -> Result<U256, StatusCode> {
    Ok(state.receiver.as_evm_word())
}

#[inline]
pub fn origin(_state: &mut ExecutionState, system: &System<impl Runtime>)  -> Result<U256, StatusCode> {
    let origin_addr = system
        .resolve_ethereum_address(&system.rt.message().origin())
        .expect("failed to resolve origin address");
    Ok(origin_addr.as_evm_word())
}

#[inline]
pub fn call_value(_state: &mut ExecutionState, system: &System<impl Runtime>)   -> Result<U256, StatusCode> {
    Ok(U256::from(&system.rt.message().value_received()))
}

#[inline]
pub fn coinbase(state: &mut ExecutionState, _system: &System<impl Runtime>) {
    // TODO do we want to return the zero ID address, or just a plain 0?
    state.stack.push(U256::zero())
}

#[inline]
pub fn gas_price(state: &mut ExecutionState, system: &System<impl Runtime>) {
    let effective_price = system.rt.base_fee() + system.rt.message().gas_premium();
    state.stack.push(U256::from(&effective_price));
}

#[inline]
pub fn gas(state: &mut ExecutionState, system: &System<impl Runtime>) {
    state.stack.push(U256::from(system.rt.gas_available()));
}

#[inline]
pub fn timestamp(state: &mut ExecutionState, system: &System<impl Runtime>) {
    state.stack.push(U256::from(system.rt.tipset_timestamp()));
}

#[inline]
pub fn block_number(state: &mut ExecutionState, system: &System<impl Runtime>) {
    state.stack.push(U256::from(system.rt.curr_epoch()));
}

#[inline]
pub fn difficulty(state: &mut ExecutionState, _system: &System<impl Runtime>) {
    state.stack.push(U256::zero());
}

#[inline]
pub fn gas_limit(state: &mut ExecutionState, _system: &System<impl Runtime>) {
    const BLOCK_GAS_LIMIT: u64 = 10_000_000_000u64;
    state.stack.push(U256::from(BLOCK_GAS_LIMIT));
}

#[inline]
pub fn chain_id(state: &mut ExecutionState, _system: &System<impl Runtime>) {
    state.stack.push(U256::from(chainid::CHAINID));
}

#[inline]
pub fn base_fee(state: &mut ExecutionState, system: &System<impl Runtime>) {
    state.stack.push(U256::from(&system.rt.base_fee()))
}
