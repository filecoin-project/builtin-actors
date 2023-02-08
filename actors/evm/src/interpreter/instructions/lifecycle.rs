use fil_actors_evm_shared::address::EthAddress;
use fil_actors_evm_shared::uints::U256;
use fil_actors_runtime::ActorError;
use fil_actors_runtime::EAM_ACTOR_ADDR;
use fil_actors_runtime::{deserialize_block, extract_send_result};
use fvm_ipld_encoding::ipld_block::IpldBlock;
use fvm_shared::sys::SendFlags;
use fvm_shared::MethodNum;
use fvm_shared::METHOD_SEND;
use fvm_shared::{address::Address, econ::TokenAmount};

use crate::ext::eam;
use crate::interpreter::Output;
use crate::EVM_CONTRACT_SELFDESTRUCT_FAILED;

use super::memory::{get_memory_region, MemoryRegion};
use {
    crate::interpreter::{ExecutionState, System},
    fil_actors_runtime::runtime::Runtime,
};

#[inline]
pub fn create(
    state: &mut ExecutionState,
    system: &mut System<impl Runtime>,
    value: U256,
    offset: U256,
    size: U256,
) -> Result<U256, ActorError> {
    if system.readonly {
        return Err(ActorError::read_only("create called while read-only".into()));
    }

    let ExecutionState { stack: _, memory, .. } = state;

    let value = TokenAmount::from(&value);
    if value > system.rt.current_balance() {
        return Ok(U256::zero());
    }
    let input_region = get_memory_region(memory, offset, size)?;

    let input_data = if let Some(MemoryRegion { offset, size }) = input_region {
        &memory[offset..][..size.get()]
    } else {
        &[]
    };

    // We increment the nonce earlier than in the EVM. See the comment in `create2` for details.
    let nonce = system.increment_nonce();
    let params = eam::CreateParams { code: input_data.to_vec(), nonce };
    create_init(system, IpldBlock::serialize_cbor(&params)?, eam::CREATE_METHOD_NUM, value)
}

pub fn create2(
    state: &mut ExecutionState,
    system: &mut System<impl Runtime>,
    endowment: U256,
    offset: U256,
    size: U256,
    salt: U256,
) -> Result<U256, ActorError> {
    if system.readonly {
        return Err(ActorError::read_only("create2 called while read-only".into()));
    }

    let ExecutionState { stack: _, memory, .. } = state;

    let endowment = TokenAmount::from(&endowment);
    if endowment > system.rt.current_balance() {
        return Ok(U256::zero());
    }

    let input_region = get_memory_region(memory, offset, size)?;

    // BE encoded array
    let salt: [u8; 32] = salt.into();

    let input_data = if let Some(MemoryRegion { offset, size }) = input_region {
        &memory[offset..][..size.get()]
    } else {
        &[]
    };
    let params = eam::Create2Params { code: input_data.to_vec(), salt };

    // We increment the nonce earlier than in the EVM, but this is unlikely to cause issues:
    //
    // 1. Like the EVM, we increment the nonce on address conflict (effectively "skipping" the
    //    address).
    // 2. Like the EVM, we increment the nonce even if the target contract runs out of gas.
    // 4. Like the EVM, we don't increment the nonce if the caller doesn't have enough funds to
    //    cover the endowment.
    // 4. Unlike the EVM, we increment the nonce if contract creation fails because we're at the max
    //    stack depth. However, given that there are other ways to increment the nonce without
    //    deploying a contract (e.g., 2), this shouldn't be an issue.
    system.increment_nonce();
    create_init(system, IpldBlock::serialize_cbor(&params)?, eam::CREATE2_METHOD_NUM, endowment)
}

/// call into Ethereum Address Manager to make the new account
#[inline]
fn create_init(
    system: &mut System<impl Runtime>,
    params: Option<IpldBlock>,
    method: MethodNum,
    value: TokenAmount,
) -> Result<U256, ActorError> {
    // Apply EIP-150
    let gas_limit = (63 * system.rt.gas_available()) / 64;

    // send bytecode & params to EAM to generate the address and contract
    let ret =
        system.send(&EAM_ACTOR_ADDR, method, params, value, Some(gas_limit), SendFlags::default());

    Ok(match ret {
        Ok(eam_ret) => {
            let ret: eam::CreateReturn = deserialize_block(eam_ret)?;
            ret.eth_address.as_evm_word()
        }
        Err(_) => U256::zero(),
    })
}

#[inline]
pub fn selfdestruct(
    _state: &mut ExecutionState,
    system: &mut System<impl Runtime>,
    beneficiary: U256,
) -> Result<Output, ActorError> {
    use crate::interpreter::output::Outcome;

    if system.readonly {
        return Err(ActorError::read_only("selfdestruct called while read-only".into()));
    }

    // Try to give funds to the beneficiary. If this fails, we abort the entire call. This can only
    // fail if:
    //
    // 1. The target address is an embedded ID address and said actor doesn't exist.
    // 2. We're at the maximum call depth.
    // 3. This call would cause us to exceed some system limit (e.g., a memory limit).
    let beneficiary: EthAddress = beneficiary.into();
    let beneficiary: Address = beneficiary.into();
    let balance = system.rt.current_balance();
    extract_send_result(system.rt.send_simple(&beneficiary, METHOD_SEND, None, balance)).map_err(
        |e| {
            ActorError::unchecked(
                EVM_CONTRACT_SELFDESTRUCT_FAILED,
                format!(
                    "failed to transfer funds to beneficiary {beneficiary} on SELFDESTRUCT: {e}"
                ),
            )
        },
    )?;

    // Now mark ourselves as deleted.
    system.mark_selfdestructed();

    // And "return".
    //
    // 1. In the constructor, this will set our code to "empty". This is correct.
    // 2. Otherwise, we'll successfully return nothing to the caller.
    Ok(Output { outcome: Outcome::Return, return_data: Vec::new() })
}

#[cfg(test)]
mod tests {
    use crate::evm_unit_test;
    use crate::ext::eam;

    use fil_actors_runtime::EAM_ACTOR_ADDR;
    use fil_actors_evm_shared::uints::U256;
    use fvm_shared::error::{ExitCode, ErrorNumber};
    use fvm_ipld_encoding::ipld_block::IpldBlock;
    use fvm_shared::sys::SendFlags;


    #[test]
    fn test_create() {
        let ret_addr = EthAddress(hex_literal::hex!("CAFEB0BA00000000000000000000000000000000"));

        evm_unit_test! {
            (rt) {
                rt.set_balance(TokenAmount::from_atto(1_000_000));

                let code = vec![0x01, 0x02, 0x03, 0x04];
                let nonce = 1;
                let create_params = eam::CreateParams { code, nonce };
                let create_ret = eam::CreateReturn {
                    actor_id: 12345,
                    eth_address: ret_addr,
                    robust_address: Some((&ret_addr).try_into().unwrap()),
                };

                rt.expect_gas_available(10_000_000_000);
                rt.expect_send(
                    EAM_ACTOR_ADDR,
                    eam::CREATE_METHOD_NUM,
                    IpldBlock::serialize_cbor(&create_params).unwrap(),
                    TokenAmount::from_atto(1234),
                    Some(63 * 10_000_000_000 / 64),
                    SendFlags::empty(),
                    IpldBlock::serialize_cbor(&create_ret).unwrap(),
                    ExitCode::OK,
                    None,
                );
            }
            (m) {
                // input data
                PUSH4; 0x01; 0x02; 0x03; 0x04;
                PUSH0;
                MSTORE;
                // the deed
                CREATE;
            }
            m.state.stack.push(U256::from(4)).unwrap();    // input size
            m.state.stack.push(U256::from(28)).unwrap();   // input offset
            m.state.stack.push(U256::from(1234)).unwrap(); // initial value
            for _ in 0..4 {
                m.step().expect("execution step failed");
            }
            assert_eq!(m.state.stack.len(), 1);
            assert_eq!(m.state.stack.pop().unwrap(), ret_addr.as_evm_word());
        };
    }

    #[test]
    fn test_create2() {
        let ret_addr = EthAddress(hex_literal::hex!("CAFEB0BA00000000000000000000000000000000"));

        evm_unit_test! {
            (rt) {
                rt.set_balance(TokenAmount::from_atto(1_000_000));

                let code = vec![0x01, 0x02, 0x03, 0x04];
                let mut salt = [0u8; 32];
                salt[28] = 0xDE;
                salt[29] = 0xAD;
                salt[30] = 0xBE;
                salt[31] = 0xEF;
                let create_params = eam::Create2Params { code, salt };
                let create_ret = eam::CreateReturn {
                    actor_id: 12345,
                    eth_address: ret_addr,
                    robust_address: Some((&ret_addr).try_into().unwrap()),
                };

                rt.expect_gas_available(10_000_000_000);
                rt.expect_send(
                    EAM_ACTOR_ADDR,
                    eam::CREATE2_METHOD_NUM,
                    IpldBlock::serialize_cbor(&create_params).unwrap(),
                    TokenAmount::from_atto(1234),
                    Some(63 * 10_000_000_000 / 64),
                    SendFlags::empty(),
                    IpldBlock::serialize_cbor(&create_ret).unwrap(),
                    ExitCode::OK,
                    None,
                );
            }
            (m) {
                // input data
                PUSH4; 0x01; 0x02; 0x03; 0x04;
                PUSH0;
                MSTORE;
                // the deed
                CREATE2;
            }
            m.state.stack.push(U256::from(0xDEADBEEFu64)).unwrap(); // salt
            m.state.stack.push(U256::from(4)).unwrap();          // input size
            m.state.stack.push(U256::from(28)).unwrap();         // input offset
            m.state.stack.push(U256::from(1234)).unwrap();       // initial value
            for _ in 0..4 {
                m.step().expect("execution step failed");
            }
            assert_eq!(m.state.stack.len(), 1);
            assert_eq!(m.state.stack.pop().unwrap(), ret_addr.as_evm_word());
        };
    }

    #[test]
    fn test_create_fail_eam() {
        evm_unit_test! {
            (rt) {
                rt.set_balance(TokenAmount::from_atto(1_000_000));

                let code = vec![0x01, 0x02, 0x03, 0x04];
                let nonce = 1;
                let create_params = eam::CreateParams { code, nonce };

                rt.expect_gas_available(10_000_000_000);
                rt.expect_send(
                    EAM_ACTOR_ADDR,
                    eam::CREATE_METHOD_NUM,
                    IpldBlock::serialize_cbor(&create_params).unwrap(),
                    TokenAmount::from_atto(1234),
                    Some(63 * 10_000_000_000 / 64),
                    SendFlags::empty(),
                    None,
                    ExitCode::USR_FORBIDDEN,
                    None,
                );
            }
            (m) {
                // input data
                PUSH4; 0x01; 0x02; 0x03; 0x04;
                PUSH0;
                MSTORE;
                // the deed
                CREATE;
            }
            m.state.stack.push(U256::from(4)).unwrap();    // input size
            m.state.stack.push(U256::from(28)).unwrap();   // input offset
            m.state.stack.push(U256::from(1234)).unwrap(); // initial value
            for _ in 0..4 {
                m.step().expect("execution step failed");
            }
            assert_eq!(m.state.stack.len(), 1);
            assert_eq!(m.state.stack.pop().unwrap(), U256::from(0));
        };
    }

    #[test]
    fn test_create_fail_nofunds() {
        evm_unit_test! {
            (rt) {
                rt.set_balance(TokenAmount::from_atto(1));
            }
            (m) {
                // input data
                PUSH4; 0x01; 0x02; 0x03; 0x04;
                PUSH0;
                MSTORE;
                // the deed
                CREATE;
            }
            m.state.stack.push(U256::from(4)).unwrap();    // input size
            m.state.stack.push(U256::from(28)).unwrap();   // input offset
            m.state.stack.push(U256::from(1234)).unwrap(); // initial value
            for _ in 0..4 {
                m.step().expect("execution step failed");
            }
            assert_eq!(m.state.stack.len(), 1);
            assert_eq!(m.state.stack.pop().unwrap(), U256::from(0));
        };
    }

    #[test]
    fn test_create_err() {
        evm_unit_test! {
            (rt) {
                rt.set_balance(TokenAmount::from_atto(1_000_000));

                let code = vec![0x01, 0x02, 0x03, 0x04];
                let nonce = 1;
                let create_params = eam::CreateParams { code, nonce };

                rt.expect_gas_available(10_000_000_000);
                rt.expect_send(
                    EAM_ACTOR_ADDR,
                    eam::CREATE_METHOD_NUM,
                    IpldBlock::serialize_cbor(&create_params).unwrap(),
                    TokenAmount::from_atto(1234),
                    Some(63 * 10_000_000_000 / 64),
                    SendFlags::empty(),
                    None,
                    ExitCode::OK,
                    Some(ErrorNumber::IllegalOperation),
                );
            }
            (m) {
                // input data
                PUSH4; 0x01; 0x02; 0x03; 0x04;
                PUSH0;
                MSTORE;
                // the deed
                CREATE;
            }
            m.state.stack.push(U256::from(4)).unwrap();    // input size
            m.state.stack.push(U256::from(28)).unwrap();   // input offset
            m.state.stack.push(U256::from(1234)).unwrap(); // initial value
            for _ in 0..4 {
                m.step().expect("execution step failed");
            }
            assert_eq!(m.state.stack.len(), 1);
            assert_eq!(m.state.stack.pop().unwrap(), U256::from(0));
        };
    }
}
