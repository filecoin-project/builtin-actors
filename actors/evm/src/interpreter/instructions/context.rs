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
            // The EVM allows fetching blockhashes from the 256 _previous_ blocks, not including the
            // current. The FVM allows fetching block CIDs from the last 899 epochs, not including
            // the current epoch.
            let curr_epoch = system.rt.curr_epoch();
            height >= curr_epoch - 256 && height < curr_epoch
        })
        .and_then(|height| system.rt.tipset_cid(height).ok())
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
    // Eth zero address, beneficiary of the current block doesn't make much sense in Filecoin due to multiple winners in each block.
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
    // NOTE: Panics if current epoch is negative, which should never happen in the network
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
    use fvm_shared::address::Address as FilAddress;
    use fil_actors_runtime::EAM_ACTOR_ID;

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

    #[test]
    fn test_number() {
        for epoch in [1234, i64::MAX, 0, 1] {
            evm_unit_test! {
                (rt) {
                    rt.set_epoch(epoch);
                }
                (m) {
                    NUMBER;
                }
                m.step().expect("execution step failed");
                assert_eq!(m.state.stack.len(), 1);
                assert_eq!(m.state.stack.pop().unwrap(), U256::from(epoch));
            };
        }
    }

    #[test]
    fn test_chainid() {
        for chainid in [31415, 3141, 0, 1] {
            evm_unit_test! {
                (rt) {
                    rt.chain_id = chainid.into();
                }
                (m) {
                    CHAINID;
                }
                m.step().expect("execution step failed");
                assert_eq!(m.state.stack.len(), 1);
                assert_eq!(m.state.stack.pop().unwrap(), U256::from(chainid));
            };
        }
    }

    #[test]
    fn test_basefee() {
        for basefee in [12345, u128::MAX, 0, 1].map(U256::from) {
            evm_unit_test! {
                (rt) {
                    rt.base_fee = TokenAmount::from(&basefee);
                }
                (m) {
                    BASEFEE;
                }
                m.step().expect("execution step failed");
                assert_eq!(m.state.stack.len(), 1);
                assert_eq!(m.state.stack.pop().unwrap(), basefee);
            };
        }
    }

    #[test]
    fn test_coinbase() {
        evm_unit_test! {
            (m) {
                COINBASE;
            }
            m.step().expect("execution step failed");
            assert_eq!(m.state.stack.len(), 1);
            assert_eq!(m.state.stack.pop().unwrap(), U256::ZERO);
        };
    }

    #[test]
    fn test_timestamp() {
        evm_unit_test! {
            (rt) {
                rt.tipset_timestamp = 12345;
            }
            (m) {
                TIMESTAMP;
            }
            m.step().expect("execution step failed");
            assert_eq!(m.state.stack.len(), 1);
            assert_eq!(m.state.stack.pop().unwrap(), U256::from(12345));
        };
    }

    #[test]
    fn test_prevrandao() {
        let epoch = 1234;
        evm_unit_test! {
            (rt) {
                rt.set_epoch(epoch);
                rt.expect_get_randomness_from_beacon(fil_actors_runtime::runtime::DomainSeparationTag::EvmPrevRandao, epoch, Vec::from(*b"prevrandao"), [0xff; 32]);
            }
            (m) {
                PREVRANDAO;
            }
            m.step().expect("execution step failed");
            assert_eq!(m.state.stack.len(), 1);
            assert_eq!(m.state.stack.pop().unwrap(), U256::MAX);
        };
    }

    #[test]
    fn test_gas_limit() {
        for limit in [12345, 0, u64::MAX] {
            evm_unit_test! {
                (rt) {
                    rt.gas_limit = limit;
                }
                (m) {
                    GASLIMIT;
                }
                m.step().expect("execution step failed");
                assert_eq!(m.state.stack.len(), 1);
                // always block gas limit
                assert_eq!(m.state.stack.pop().unwrap(), U256::from(10_000_000_000u64));
            };
        }
    }

    #[test]
    fn test_address() {
        evm_unit_test! {
            (m) {
                ADDRESS;
            }
            let addr = EthAddress::from_id(1001);
            m.state.receiver = addr;
            m.step().expect("execution step failed");
            assert_eq!(m.state.stack.len(), 1);
            assert_eq!(m.state.stack.pop().unwrap(), addr.as_evm_word());
        };
    }

    #[test]
    fn test_origin_id() {
        let eth_addr = EthAddress::from_id(1000); // default origin in construction of rt in macro
        let fil_addr = FilAddress::new_id(1000);
        evm_unit_test! {
            (rt) {
                rt.in_call = true;
                rt.set_origin(fil_addr);
            }
            (m) {
                ORIGIN;
            }
            m.step().expect("execution step failed");
            assert_eq!(m.state.stack.len(), 1);
            assert_eq!(m.state.stack.pop().unwrap(), eth_addr.as_evm_word());
        };
    }

    #[test]
    fn test_origin_ethaddr() {
        let addr_bytes = hex_literal::hex!("FEEDFACECAFEBEEF000000000000000000001234");
        let eth_addr = EthAddress(addr_bytes);
        let fil_addr = FilAddress::new_delegated(EAM_ACTOR_ID, &addr_bytes).unwrap();
        evm_unit_test! {
            (rt) {
                rt.in_call = true;
                rt.set_origin(fil_addr);
            }
            (m) {
                ORIGIN;
            }
            m.step().expect("execution step failed");
            assert_eq!(m.state.stack.len(), 1);
            assert_eq!(m.state.stack.pop().unwrap(), eth_addr.as_evm_word());
        };
    }

    #[test]
    fn test_caller() {
        evm_unit_test! {
            (m) {
                CALLER;
            }
            let addr = EthAddress::from_id(1001);
            m.state.caller = addr;
            m.step().expect("execution step failed");
            assert_eq!(m.state.stack.len(), 1);
            assert_eq!(m.state.stack.pop().unwrap(), addr.as_evm_word());
        };
    }

    #[test]
    fn test_gas() {
        evm_unit_test! {
            (rt) {
                rt.expect_gas_available(1234000);
            }
            (m) {
                GAS;
            }
            let addr = EthAddress::from_id(1001);
            m.state.caller = addr;
            m.step().expect("execution step failed");
            assert_eq!(m.state.stack.len(), 1);
            assert_eq!(m.state.stack.pop().unwrap(), U256::from(1234000));
        };
    }

}
