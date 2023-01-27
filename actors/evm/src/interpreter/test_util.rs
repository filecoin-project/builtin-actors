#![cfg(test)]

#[macro_export]
macro_rules! do_test {
    ($rt:ident, $machine:ident, $code:expr, $body:block) => {
        use ::fvm_shared::econ::TokenAmount;
        use $crate::interpreter::{execution::Machine, system::System, Output};
        use $crate::{Bytecode, Bytes, EthAddress, ExecutionState};

        let mut $rt = MockRuntime::default();
        let mut state = ExecutionState::new(
            EthAddress::from_id(1000),
            EthAddress::from_id(1000),
            TokenAmount::from_atto(0),
            Bytes::default(),
        );

        let mut system = System::new(&mut $rt, false);
        let bytecode = Bytecode::new($code);
        let mut $machine = Machine {
            system: &mut system,
            state: &mut state,
            bytecode: &bytecode,
            pc: 0,
            output: Output::default(),
        };

        $body
    };
}
