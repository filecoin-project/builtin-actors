#[macro_export]
macro_rules! evm_instruction {
    ($i:ident) => {
        $crate::interpreter::execution::opcodes::$i
    };
    ($i:literal) => {
        $i
    };
    ($i:expr) => {
        $i
    };
}

#[macro_export]
macro_rules! evm_unit_test {
    (($rt:ident) $init:block ($machine:ident) { $($inst:tt;)* } $($body:tt)*) => {
        use ::fil_actors_runtime::test_utils::MockRuntime;
        use ::fvm_shared::econ::TokenAmount;
        use $crate::interpreter::{execution::Machine, system::System, Output};
        use $crate::{Bytecode, EthAddress, ExecutionState};

        #[allow(unused_mut)]
        let mut $rt = MockRuntime::default();
        $rt.in_call.replace(true);
        $init

        let mut state = ExecutionState::new(
            EthAddress::from_id(1000),
            EthAddress::from_id(1000),
            TokenAmount::from_atto(0),
            Vec::new(),
        );

        let code = vec![$($crate::evm_instruction!($inst)),*];

        let mut system = System::new(&$rt, false);
        let bytecode = Bytecode::new(code);
        #[allow(unused_mut)]
        let mut $machine = Machine {
            system: &mut system,
            state: &mut state,
            bytecode: &bytecode,
            pc: 0,
            output: Output::default(),
        };

        $($body)*
    };

    (($machine:ident) { $($inst:tt;)* } $($body:tt)*) => {
        use ::fil_actors_runtime::test_utils::MockRuntime;
        use ::fvm_shared::econ::TokenAmount;
        use $crate::interpreter::{execution::Machine, system::System, Output};
        use $crate::{Bytecode, EthAddress, ExecutionState};

        let rt = MockRuntime::default();
        rt.in_call.replace(true);
        let mut state = ExecutionState::new(
            EthAddress::from_id(1000),
            EthAddress::from_id(1000),
            TokenAmount::from_atto(0),
            Vec::new(),
        );

        let code = vec![$($crate::evm_instruction!($inst)),*];

        let mut system = System::new(&rt, false);
        let bytecode = Bytecode::new(code);
        #[allow(unused_mut)]
        let mut $machine = Machine {
            system: &mut system,
            state: &mut state,
            bytecode: &bytecode,
            pc: 0,
            output: Output::default(),
        };

        $($body)*
    };
}
