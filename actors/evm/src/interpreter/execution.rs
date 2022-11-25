#![allow(dead_code)]

use fvm_shared::address::Address as FilecoinAddress;

use super::address::EthAddress;
use {
    super::instructions,
    super::opcode::OpCode,
    super::StatusCode,
    crate::interpreter::instructions::call::CallKind,
    crate::interpreter::memory::Memory,
    crate::interpreter::stack::Stack,
    crate::interpreter::{Bytecode, Output, System},
    bytes::Bytes,
    fil_actors_runtime::runtime::Runtime,
};

/// EVM execution runtime.
#[derive(Clone, Debug)]
pub struct ExecutionState {
    pub stack: Stack,
    pub memory: Memory,
    pub input_data: Bytes,
    pub return_data: Bytes,
    pub output_data: Bytes,
    /// Indicates whether the contract called SELFDESTRUCT, providing the beneficiary.
    pub selfdestroyed: Option<FilecoinAddress>,
    /// The EVM address of the caller.
    pub caller: EthAddress,
    /// The EVM address of the receiver.
    pub receiver: EthAddress,
}

impl ExecutionState {
    pub fn new(caller: EthAddress, receiver: EthAddress, input_data: Bytes) -> Self {
        Self {
            stack: Stack::new(),
            memory: Memory::default(),
            input_data,
            return_data: Default::default(),
            output_data: Bytes::new(),
            selfdestroyed: None,
            caller,
            receiver,
        }
    }
}

pub struct Machine<'r, 'a, RT: Runtime + 'a> {
    system: &'r mut System<'a, RT>,
    state: &'r mut ExecutionState,
    bytecode: &'r Bytecode,
    pc: usize,
    reverted: bool,
}

enum ControlFlow {
    Continue,
    Jump,
    Exit,
}

type Instruction<M> = fn(*mut M) -> Result<ControlFlow, StatusCode>;

macro_rules! def_opcodes {
    ($($op:ident: $body:tt)*) => {
        def_ins_raw! {
            UNDEFINED(_m) {
                Err(StatusCode::UndefinedInstruction)
            }
        }
        $(def_ins! { $op $body })*
        def_jmptable! {
            $($op)*
        }
    }
}

macro_rules! def_jmptable {
    ($($op:ident)*) => {
        const fn jmptable() -> [Instruction<Machine<'r, 'a, RT>>; 256] {
            let mut table: [Instruction<Machine::<'r, 'a, RT>>; 256] = [Machine::<'r, 'a, RT>::UNDEFINED; 256];
            $(table[OpCode::$op as usize] = Machine::<'r, 'a, RT>::$op;)*
            table
        }
    }
}

macro_rules! def_ins {
    ($ins:ident (primitive)) => {
        def_ins_primitive! { $ins }
    };

    ($ins:ident (push)) => {
        def_ins_push! { $ins }
    };

    ($ins:ident (stdfun)) => {
        def_ins_stdfun! { $ins }
    };

    ($ins:ident {=> $expr:expr}) => {
        def_ins_raw! { $ins (_m) { $expr } }
    };

    ($ins:ident {($arg:ident) => $body:block}) => {
        def_ins_raw! { $ins ($arg) $body }
    };

}

macro_rules! def_ins_raw {
    ($ins:ident ($arg:ident) $body:block) => {
        #[allow(non_snake_case)]
        fn $ins(p: *mut Self) -> Result<ControlFlow, StatusCode> {
            // SAFETY: macro ensures that mut pointer is taken directly from a mutable borrow, used once, then goes out of scope immediately after
            let $arg: &mut Self = unsafe { p.as_mut().unwrap() };
            $body
        }
    };
}

macro_rules! def_ins_primitive {
    ($ins:ident) => {
        def_ins_raw!{
            $ins (m) {
                instructions::$ins(&mut m.state.stack)?;
                Ok(ControlFlow::Continue)
            }
        }
    }
}

macro_rules! def_ins_push {
    ($ins:ident) => {
        def_ins_raw!{
            $ins (m) {
                m.pc += instructions::$ins(&mut m.state.stack, &m.bytecode[m.pc + 1..])?;
                Ok(ControlFlow::Continue)
            }
        }
    }
}

macro_rules! def_ins_stdfun {
    ($ins:ident) => {
        def_ins_raw!{
            $ins (m) {
                instructions::$ins(m.state, m.system)?;
                Ok(ControlFlow::Continue)
            }
        }
    }
}

impl<'r, 'a, RT: Runtime + 'r> Machine<'r, 'a, RT> {
    pub fn new(
        system: &'r mut System<'a, RT>,
        state: &'r mut ExecutionState,
        bytecode: &'r Bytecode,
    ) -> Self {
        Machine { system, state, bytecode, pc: 0, reverted: false }
    }

    pub fn execute(&mut self) -> Result<(), StatusCode> {
        loop {
            if self.pc >= self.bytecode.len() {
                break;
            }

            match self.step()? {
                ControlFlow::Continue => {
                    self.pc += 1;
                }
                ControlFlow::Jump => {}
                ControlFlow::Exit => {
                    break;
                }
            };
        }

        Ok(())
    }

    fn step(&mut self) -> Result<ControlFlow, StatusCode> {
        let op = OpCode::try_from(self.bytecode[self.pc])?;
        Self::JMPTABLE[op as usize](self)
    }

    def_opcodes! {
        STOP: {=> Ok(ControlFlow::Exit)}
        ADD: (primitive)
        MUL: (primitive)
        SUB: (primitive)
        DIV: (primitive)
        SDIV: (primitive)
        MOD: (primitive)
        SMOD: (primitive)
        ADDMOD: (primitive)
        MULMOD: (primitive)
        EXP: (primitive)
        SIGNEXTEND: (primitive)
        LT: (primitive)
        GT: (primitive)
        SLT: (primitive)
        SGT: (primitive)
        EQ: (primitive)
        ISZERO: (primitive)
        AND: (primitive)
        OR: (primitive)
        XOR: (primitive)
        NOT: (primitive)
        BYTE: (primitive)
        SHL: (primitive)
        SHR: (primitive)
        SAR: (primitive)

        KECCAK256: (stdfun)
        ADDRESS: (stdfun)
        BALANCE: (stdfun)
        ORIGIN: (stdfun)
        CALLER: (stdfun)
        CALLVALUE: (stdfun)
        CALLDATALOAD: (stdfun)
        CALLDATASIZE: (stdfun)
        CALLDATACOPY: (stdfun)

        CODESIZE: {(m) => {
            instructions::call::codesize(&mut m.state.stack, m.bytecode.as_ref());
            Ok(ControlFlow::Continue)
        }}

        CODECOPY: {(m) => {
            instructions::call::codecopy(m.state, m.bytecode.as_ref())?;
            Ok(ControlFlow::Continue)
        }}

        GASPRICE: {(m) => {
            instructions::context::gas_price(m.state, m.system);
            Ok(ControlFlow::Continue)
        }}

        EXTCODESIZE: {(m) => {
            instructions::ext::extcodesize(m.state, m.system)?;
            Ok(ControlFlow::Continue)
        }}

        EXTCODECOPY: {(m) => {
            instructions::ext::extcodecopy(m.state, m.system)?;
            Ok(ControlFlow::Continue)
        }}

        RETURNDATASIZE: {(m) => {
            instructions::control::returndatasize(m.state);
            Ok(ControlFlow::Continue)
        }}

        RETURNDATACOPY: {(m) => {
            instructions::control::returndatacopy(m.state)?;
            Ok(ControlFlow::Continue)
        }}

        EXTCODEHASH: {(m) => {
            instructions::ext::extcodehash(m.state, m.system)?;
            Ok(ControlFlow::Continue)
        }}

        BLOCKHASH: {(m) => {
            instructions::context::blockhash(m.state, m.system);
            Ok(ControlFlow::Continue)
        }}

        COINBASE: {(m) => {
            instructions::context::coinbase(m.state, m.system);
            Ok(ControlFlow::Continue)
        }}

        TIMESTAMP: {(m) => {
            instructions::context::timestamp(m.state, m.system);
            Ok(ControlFlow::Continue)
        }}

        NUMBER: {(m) => {
            instructions::context::block_number(m.state, m.system);
            Ok(ControlFlow::Continue)
        }}

        DIFFICULTY: {(m) => {
            instructions::context::difficulty(m.state, m.system);
            Ok(ControlFlow::Continue)
        }}

        GASLIMIT: {(m) => {
            instructions::context::gas_limit(m.state, m.system);
            Ok(ControlFlow::Continue)
        }}

        CHAINID: {(m) => {
            instructions::context::chain_id(m.state, m.system);
            Ok(ControlFlow::Continue)
        }}

        SELFBALANCE: {(m) => {
            instructions::state::selfbalance(m.state, m.system);
            Ok(ControlFlow::Continue)
        }}

        BASEFEE: {(m) => {
            instructions::context::base_fee(m.state, m.system);
            Ok(ControlFlow::Continue)
        }}

        POP: (primitive)

        MLOAD: {(m) => {
            instructions::memory::mload(m.state)?;
            Ok(ControlFlow::Continue)
        }}

        MSTORE: {(m) => {
            instructions::memory::mstore(m.state)?;
            Ok(ControlFlow::Continue)
        }}

        MSTORE8: {(m) => {
            instructions::memory::mstore8(m.state)?;
            Ok(ControlFlow::Continue)
        }}

        SLOAD: {(m) => {
            instructions::storage::sload(m.state, m.system)?;
            Ok(ControlFlow::Continue)
        }}

        SSTORE: {(m) => {
            instructions::storage::sstore(m.state, m.system)?;
            Ok(ControlFlow::Continue)
        }}

        JUMP: {(m) => {
            m.pc = instructions::control::jump(&mut m.state.stack, m.bytecode)?;
            Ok(ControlFlow::Jump)
        }}

        JUMPI: {(m) => {
            if let Some(dest) = instructions::control::jumpi(&mut m.state.stack, m.bytecode)? {
                m.pc = dest;
                Ok(ControlFlow::Jump)
            } else {
                Ok(ControlFlow::Continue)
            }
        }}

        PC: {(m) => {
            instructions::control::pc(&mut m.state.stack, m.pc);
            Ok(ControlFlow::Continue)
        }}

        MSIZE: {(m) => {
            instructions::memory::msize(m.state);
            Ok(ControlFlow::Continue)
        }}

        GAS: {(m) => {
            instructions::context::gas(m.state, m.system);
            Ok(ControlFlow::Continue)
        }}

        JUMPDEST: {=> Ok(ControlFlow::Continue)} // noop marker opcode for valid jumps addresses

        PUSH1: (push)
        PUSH2: (push)
        PUSH3: (push)
        PUSH4: (push)
        PUSH5: (push)
        PUSH6: (push)
        PUSH7: (push)
        PUSH8: (push)
        PUSH9: (push)
        PUSH10: (push)
        PUSH11: (push)
        PUSH12: (push)
        PUSH13: (push)
        PUSH14: (push)
        PUSH15: (push)
        PUSH16: (push)
        PUSH17: (push)
        PUSH18: (push)
        PUSH19: (push)
        PUSH20: (push)
        PUSH21: (push)
        PUSH22: (push)
        PUSH23: (push)
        PUSH24: (push)
        PUSH25: (push)
        PUSH26: (push)
        PUSH27: (push)
        PUSH28: (push)
        PUSH29: (push)
        PUSH30: (push)
        PUSH31: (push)
        PUSH32: (push)

        DUP1: (primitive)
        DUP2: (primitive)
        DUP3: (primitive)
        DUP4: (primitive)
        DUP5: (primitive)
        DUP6: (primitive)
        DUP7: (primitive)
        DUP8: (primitive)
        DUP9: (primitive)
        DUP10: (primitive)
        DUP11: (primitive)
        DUP12: (primitive)
        DUP13: (primitive)
        DUP14: (primitive)
        DUP15: (primitive)
        DUP16: (primitive)

        SWAP1: (primitive)
        SWAP2: (primitive)
        SWAP3: (primitive)
        SWAP4: (primitive)
        SWAP5: (primitive)
        SWAP6: (primitive)
        SWAP7: (primitive)
        SWAP8: (primitive)
        SWAP9: (primitive)
        SWAP10: (primitive)
        SWAP11: (primitive)
        SWAP12: (primitive)
        SWAP13: (primitive)
        SWAP14: (primitive)
        SWAP15: (primitive)
        SWAP16: (primitive)

        LOG0: {(m) => {
            instructions::log::log(m.state, m.system, 0)?;
            Ok(ControlFlow::Continue)
        }}

        LOG1: {(m) => {
            instructions::log::log(m.state, m.system, 1)?;
            Ok(ControlFlow::Continue)
        }}

        LOG2: {(m) => {
            instructions::log::log(m.state, m.system, 2)?;
            Ok(ControlFlow::Continue)
        }}

        LOG3: {(m) => {
            instructions::log::log(m.state, m.system, 3)?;
            Ok(ControlFlow::Continue)
        }}

        LOG4: {(m) => {
            instructions::log::log(m.state, m.system, 4)?;
            Ok(ControlFlow::Continue)
        }}

        CREATE: {(m) => {
            instructions::lifecycle::create(m.state, m.system)?;
            Ok(ControlFlow::Continue)
        }}

        CALL: {(m) => {
            instructions::call::call(m.state, m.system, CallKind::Call)?;
            Ok(ControlFlow::Continue)
        }}

        CALLCODE: {(m) => {
            instructions::call::call(m.state, m.system, CallKind::CallCode)?;
            Ok(ControlFlow::Continue)
        }}

        RETURN: {(m) => {
            instructions::control::ret(m.state)?;
            Ok(ControlFlow::Exit)
        }}

        DELEGATECALL: {(m) => {
            instructions::call::call(m.state, m.system, CallKind::DelegateCall)?;
            Ok(ControlFlow::Continue)
        }}

        CREATE2: {(m) => {
            instructions::lifecycle::create2(m.state, m.system)?;
            Ok(ControlFlow::Continue)
        }}

        STATICCALL: {(m) => {
            instructions::call::call(m.state, m.system, CallKind::StaticCall)?;
            Ok(ControlFlow::Continue)
        }}

        REVERT: {(m) => {
            instructions::control::ret(m.state)?;
            m.reverted = true;
            Ok(ControlFlow::Exit)
        }}

        INVALID: {=> Err(StatusCode::InvalidInstruction)}

        SELFDESTRUCT: {(m) => {
            instructions::lifecycle::selfdestruct(m.state, m.system)?;
            Ok(ControlFlow::Exit) // selfdestruct halts the current context
        }}
    }

    const JMPTABLE: [Instruction<Machine<'r, 'a, RT>>; 256] = Machine::<'r, 'a, RT>::jmptable();
}

pub fn execute(
    bytecode: &Bytecode,
    runtime: &mut ExecutionState,
    system: &mut System<impl Runtime>,
) -> Result<Output, StatusCode> {
    let mut m = Machine::new(system, runtime, bytecode);
    m.execute()?;
    Ok(Output {
        reverted: m.reverted,
        status_code: StatusCode::Success,
        output_data: m.state.output_data.clone(),
        selfdestroyed: m.state.selfdestroyed,
    })
}
