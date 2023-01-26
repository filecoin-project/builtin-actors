#![cfg(test)]

use fil_actors_runtime::test_utils::*;
use fvm_shared::econ::TokenAmount;

use crate::interpreter::address::*;
use crate::interpreter::bytecode::*;
use crate::interpreter::execution::*;
use crate::interpreter::output::*;
use crate::interpreter::system::*;

use bytes::Bytes;

pub type TestSystem<'rt> = System<'rt, MockRuntime>;
pub type TestMachine<'machine, 'rt> = Machine<'machine, 'rt, MockRuntime>;

pub struct Tester<'rt> {
    pub system: TestSystem<'rt>,
    pub state: ExecutionState,
    pub bytecode: Bytecode,
}

impl<'rt> Tester<'rt> {
    pub fn machine<'m>(&'rt mut self) -> TestMachine<'m, 'rt> {
        TestMachine {
            system: &mut self.system,
            state: &mut self.state,
            bytecode: &self.bytecode,
            pc: 0,
            output: Output::default(),
        }
    }

    pub fn init(rt: &'rt mut MockRuntime, bytecode: Vec<u8>) -> Tester<'rt> {
        let state = ExecutionState::new(
            EthAddress::from_id(1000),
            EthAddress::from_id(1000),
            TokenAmount::from_atto(0),
            Bytes::default(),
        );
        Tester { state, system: TestSystem::new(rt, false), bytecode: Bytecode::new(bytecode) }
    }
}

#[macro_export]
macro_rules! do_test {
    ($tester:ident, $machine:ident, $code:expr, $body:block) => {
        let mut rt = MockRuntime::default();
        let mut $tester = Tester::init(&mut rt, $code);
        let mut $machine = $tester.machine();

        $body
    };
}
