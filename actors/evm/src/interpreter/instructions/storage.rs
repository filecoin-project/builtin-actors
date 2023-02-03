use fil_actors_evm_shared::uints::U256;
use fil_actors_runtime::ActorError;

use {
    crate::interpreter::{ExecutionState, System},
    fil_actors_runtime::runtime::Runtime,
};

#[inline]
pub fn sload(
    _state: &mut ExecutionState,
    system: &mut System<impl Runtime>,
    location: U256,
) -> Result<U256, ActorError> {
    // get from storage and place on stack
    system.get_storage(location)
}

#[inline]
pub fn sstore(
    _state: &mut ExecutionState,
    system: &mut System<impl Runtime>,
    key: U256,
    value: U256,
) -> Result<(), ActorError> {
    if system.readonly {
        return Err(ActorError::read_only("store called while read-only".into()));
    }

    system.set_storage(key, value)
}

#[cfg(test)]
mod tests {
    use fil_actors_evm_shared::uints::U256;

    use crate::evm_unit_test;

    #[test]
    fn test_sload() {
        // happy path
        evm_unit_test! {
            (m) {
                SLOAD;
            }
            m.system.set_storage(U256::from(0), U256::from(0x42)).unwrap();
            m.state.stack.push(U256::from(0)).unwrap();
            let result = m.step();
            assert!(result.is_ok(), "execution step failed");
            assert_eq!(m.state.stack.len(), 1);
            assert_eq!(m.state.stack.pop().unwrap(), U256::from(0x42));
        };
    }

    #[test]
    fn test_sload_oob() {
        // oob access -- it is a zero
        evm_unit_test! {
            (m) {
                SLOAD;
            }
            m.state.stack.push(U256::from(1234)).unwrap();
            let result = m.step();
            assert!(result.is_ok(), "execution step failed");
            assert_eq!(m.state.stack.len(), 1);
            assert_eq!(m.state.stack.pop().unwrap(), U256::from(0));
        };
    }
}
