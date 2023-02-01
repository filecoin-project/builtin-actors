use fil_actors_runtime::ActorError;
use fvm_shared::address::Address;

use crate::U256;
use {
    crate::interpreter::address::EthAddress,
    crate::interpreter::{ExecutionState, System},
    fil_actors_runtime::runtime::Runtime,
};

#[inline]
pub fn balance(
    _state: &mut ExecutionState,
    system: &System<impl Runtime>,
    actor: U256,
) -> Result<U256, ActorError> {
    let addr: EthAddress = actor.into();
    let addr: Address = addr.into();

    let balance = system
        .rt
        .resolve_address(&addr)
        .and_then(|id| system.rt.actor_balance(id).as_ref().map(U256::from))
        .unwrap_or_default();

    Ok(balance)
}

#[inline]
pub fn selfbalance(
    _state: &mut ExecutionState,
    system: &System<impl Runtime>,
) -> Result<U256, ActorError> {
    // Returns native FIL balance of the receiver. Value precision is identical to Ethereum, so
    // no conversion needed (atto, 1e18).
    Ok(U256::from(&system.rt.current_balance()))
}

#[cfg(test)]
mod test {
    use fvm_shared::address::Address;

    use crate::{
        evm_unit_test,
        interpreter::{address::EthAddress, U256},
    };

    #[test]
    fn balance_basic() {
        for balance in [0, 1234, u64::MAX as u128, u128::MAX] {
            for (has_id, addr) in [
                (true, EthAddress::from_id(1111).as_evm_word()), // eth encoded IDs are OK
                (true, EthAddress([0xff; 20]).as_evm_word()),
                (true, EthAddress([0xab; 20]).as_evm_word()),
                (false, EthAddress([0xff; 20]).as_evm_word()),
                (false, EthAddress([0xab; 20]).as_evm_word()),
                (false, U256::MAX),
            ] {
                evm_unit_test! {
                    (rt, m) {
                        PUSH0;
                        MLOAD;
                        BALANCE;
                    }

                    m.system.rt.in_call = true;

                    let id_address = 1111;
                    if has_id {
                        let addr: EthAddress = addr.into();
                        m.system.rt.add_id_address(addr.into(), Address::new_id(id_address))
                    }
                    m.system.rt.actor_balances.insert(id_address, TokenAmount::from_atto(balance));

                    m.state.memory.grow(32);
                    m.state.memory[..32].copy_from_slice(&addr.to_bytes());

                    m.step().expect("execution step failed");
                    m.step().expect("execution step failed");
                    m.step().expect("execution step failed");

                    assert_eq!(m.state.stack.len(), 1);
                    if has_id {
                        assert_eq!(m.state.stack.pop().unwrap(), U256::from(balance));
                    } else {
                        assert_eq!(m.state.stack.pop().unwrap(), U256::from(0));
                    }
                };
            }
        }
    }

    #[test]
    fn balance_invalid_input() {
        let id = 0xad;
        let balance = 1234;

        let mut buf = EthAddress::from_id(id).as_evm_word().to_bytes();
        // first bytes should be ignored silently
        buf[..12].copy_from_slice(&[0xff; 12]);
        let addr = U256::from(buf);

        evm_unit_test! {
            (rt, m) {
                PUSH0;
                MLOAD;
                BALANCE;
            }

            m.system.rt.in_call = true;

            // 0xff id address gets balance
            m.system.rt.actor_balances.insert(id as u64, TokenAmount::from_atto(balance));

            m.state.memory.grow(32);
            m.state.memory[..32].copy_from_slice(&addr.to_bytes());

            m.step().expect("execution step failed");
            m.step().expect("execution step failed");
            m.step().expect("execution step failed");

            assert_eq!(m.state.stack.len(), 1);
            // balance should be set properly
            assert_eq!(m.state.stack.pop().unwrap(), U256::from(balance));
        };
    }

    #[test]
    fn selfbalance_basic() {
        for i in 0..256 {
            let balance = U256::ONE << i;
            evm_unit_test! {
                (rt, m) {
                    SELFBALANCE;
                }

                m.system.rt.in_call = true;
                m.system.rt.add_balance(TokenAmount::from(&balance));

                m.step().expect("execution step failed");

                assert_eq!(m.state.stack.len(), 1);
                assert_eq!(m.state.stack.pop().unwrap(), balance);
            };
        }
    }
}
