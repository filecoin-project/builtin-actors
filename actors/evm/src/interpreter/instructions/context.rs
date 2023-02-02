use fil_actors_evm_shared::uints::U256;
use fil_actors_runtime::ActorError;
use fvm_shared::clock::ChainEpoch;

use crate::EVM_WORD_SIZE;

use {
    crate::interpreter::{ExecutionState, System},
    fil_actors_runtime::runtime::Runtime,
};

#[inline]
pub fn blockhash(
    _state: &mut ExecutionState,
    system: &System<impl Runtime>,
    bn: U256,
) -> Result<U256, ActorError> {
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
            if hash.len() > EVM_WORD_SIZE {
                hash = &hash[..EVM_WORD_SIZE]
            }
            U256::from_big_endian(hash)
        })
        .unwrap_or_default();
    Ok(result)
}

#[inline]
pub fn caller(state: &mut ExecutionState, _: &System<impl Runtime>) -> Result<U256, ActorError> {
    Ok(state.caller.as_evm_word())
}

#[inline]
pub fn address(
    state: &mut ExecutionState,
    _system: &System<impl Runtime>,
) -> Result<U256, ActorError> {
    Ok(state.receiver.as_evm_word())
}

#[inline]
pub fn origin(
    _state: &mut ExecutionState,
    system: &System<impl Runtime>,
) -> Result<U256, ActorError> {
    let origin_addr = system
        .resolve_ethereum_address(&system.rt.message().origin())
        .expect("failed to resolve origin address");
    Ok(origin_addr.as_evm_word())
}

#[inline]
pub fn call_value(
    state: &mut ExecutionState,
    _system: &System<impl Runtime>,
) -> Result<U256, ActorError> {
    Ok(U256::from(&state.value_received))
}

#[inline]
pub fn coinbase(
    _state: &mut ExecutionState,
    _system: &System<impl Runtime>,
) -> Result<U256, ActorError> {
    // TODO do we want to return the zero ID address, or just a plain 0?
    Ok(U256::zero())
}

#[inline]
pub fn gas_price(
    _state: &mut ExecutionState,
    system: &System<impl Runtime>,
) -> Result<U256, ActorError> {
    let effective_price = system.rt.base_fee() + system.rt.message().gas_premium();
    Ok(U256::from(&effective_price))
}

#[inline]
pub fn gas(_state: &mut ExecutionState, system: &System<impl Runtime>) -> Result<U256, ActorError> {
    Ok(U256::from(system.rt.gas_available()))
}

#[inline]
pub fn timestamp(
    _state: &mut ExecutionState,
    system: &System<impl Runtime>,
) -> Result<U256, ActorError> {
    Ok(U256::from(system.rt.tipset_timestamp()))
}

#[inline]
pub fn block_number(
    _state: &mut ExecutionState,
    system: &System<impl Runtime>,
) -> Result<U256, ActorError> {
    Ok(U256::from(system.rt.curr_epoch()))
}

/// EIP-4399: DIFFICULTY -> PREVRANDAO
#[inline]
pub fn prevrandao(
    _state: &mut ExecutionState,
    system: &mut System<impl Runtime>,
) -> Result<U256, ActorError> {
    // NOTE: Filecoin beacon randomness is expected to fall outside of the `2^64` reserved range, following PREVRANDAO's assumptions.
    // NOTE: EVM uses previous RANDAO value in this opcode since the _current_ RANDAO for them runs on the beacon chain's state
    //      and wont be finalized till the end of a block. Filecoin's chain randomness is generated _before_ any contract is run, so we instead
    //      grab randomness from the current epoch.
    system.get_randomness().map(|v| U256::from(*v))
}

#[inline]
pub fn gas_limit(
    _state: &mut ExecutionState,
    _system: &System<impl Runtime>,
) -> Result<U256, ActorError> {
    const BLOCK_GAS_LIMIT: u64 = 10_000_000_000u64;
    Ok(U256::from(BLOCK_GAS_LIMIT))
}

#[inline]
pub fn chain_id(
    _state: &mut ExecutionState,
    system: &System<impl Runtime>,
) -> Result<U256, ActorError> {
    Ok(U256::from_u64(system.rt.chain_id().into()))
}

#[inline]
pub fn base_fee(
    _state: &mut ExecutionState,
    system: &System<impl Runtime>,
) -> Result<U256, ActorError> {
    Ok(U256::from(&system.rt.base_fee()))
}

#[cfg(test)]
mod tests {
    use crate::evm_unit_test;
    use cid::Cid;
    use fil_actors_evm_shared::uints::U256;
    use fvm_ipld_encoding::{DAG_CBOR, IPLD_RAW};

    #[test]
    fn test_blockhash() {
        // truncate to 32 bytes
        let counting_byte_hash: Vec<u8> = (0..40u8).collect();
        let long_unknown =
            Cid::new_v1(IPLD_RAW, multihash::Multihash::wrap(0, &counting_byte_hash).unwrap());
        let long_expect = counting_byte_hash[..32].try_into().unwrap();
        // multihash code ignored
        let cbor_odd_hash =
            Cid::new_v1(DAG_CBOR, multihash::Multihash::wrap(123, &[0xfe; 32]).unwrap());
        let cbor_odd_expect = [0xfe; 32];

        let nothing = [0; 32];

        for (current, getting, insert, expect, test) in [
            (
                12345,
                12340u16,
                Some(long_unknown),
                long_expect,
                "truncated tipset hash, (first 32 bytes)",
            ),
            (1234, 1230u16, Some(cbor_odd_hash), cbor_odd_expect, "normal-ish tipset"),
            (123, 222u16, None, nothing, "future tipset"),
            (1234, 123u16, None, nothing, "requested older than finality (256)"),
        ] {
            let [a, b] = getting.to_be_bytes();
            evm_unit_test! {
                (rt) {
                    rt.in_call = true;
                    rt.set_epoch(current);
                    rt.tipset_cids.resize(current as usize, Cid::default());
                    if let Some(cid) = insert {
                        rt.tipset_cids[getting as usize] = cid;
                    }
                }
                (m) {
                    PUSH2;
                    {a};
                    {b};
                    BLOCKHASH;
                }
                m.step().expect("execution step failed");
                m.step().expect("execution step failed");
                assert_eq!(m.state.stack.len(), 1);
                assert_eq!(m.state.stack.pop().unwrap(), U256::from(expect), "{}", test);
            };
        }
    }

    #[test]
    fn test_callvalue() {
        evm_unit_test! {
            (m) {
                CALLVALUE;
            }
            m.state.value_received = TokenAmount::from_atto(123);
            let result = m.step();
            assert!(result.is_ok(), "execution step failed");
            assert_eq!(m.state.stack.len(), 1);
            assert_eq!(m.state.stack.pop().unwrap(), U256::from(123));
        };
    }
}
