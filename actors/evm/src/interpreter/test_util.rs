#![cfg(test)]

use fil_actors_runtime::test_utils::*;
use fvm_shared::econ::TokenAmount;

use crate::interpreter::address::*;
use crate::interpreter::bytecode::*;
use crate::interpreter::execution::*;
use crate::interpreter::output::*;
use crate::interpreter::system::*;

use bytes::Bytes;

pub type TestSystem<'r> = System<'r, MockRuntime>;
pub type TestMachine<'a, 'r> = Machine<'a, 'r, MockRuntime>;

pub struct TestEnv<'r> {
    pub system: TestSystem<'r>,
    pub state: ExecutionState,
    pub bytecode: Bytecode,
}

pub struct Tester<'a, 'r> {
    pub m: TestMachine<'a, 'r>,
    pub env: TestEnv<'r>,
}

pub fn rt() -> MockRuntime {
    MockRuntime::default()
}

pub fn env<'r>(rt: &'r mut MockRuntime, code: Vec<u8>) -> TestEnv<'r> {
    let system = TestSystem::new(rt, false);
    let state = ExecutionState::new(
        EthAddress::from_id(1000),
        EthAddress::from_id(1000),
        TokenAmount::from_atto(0),
        Bytes::default(),
    );
    let bytecode = Bytecode::new(code);
    TestEnv { system, state, bytecode }
}

pub fn machine<'a, 'r>(env: &'a mut TestEnv<'r>) -> TestMachine<'a, 'r> {
    TestMachine {
        system: &mut env.system,
        state: &mut env.state,
        bytecode: &env.bytecode,
        pc: 0,
        output: Output::default(),
    }
}
