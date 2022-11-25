#![allow(dead_code)]

use fvm_shared::address::Address as FilecoinAddress;

use super::address::EthAddress;
use {
    super::instructions,
    super::opcode::OpCode,
    super::StatusCode,
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
    ($ins:ident {primop}) => {
        def_ins_primop! { $ins }
    };

    ($ins:ident {push}) => {
        def_ins_push! { $ins }
    };

    ($ins:ident {std}) => {
        def_ins_std! { $ins }
    };

    ($ins:ident {code}) => {
        def_ins_code! { $ins }
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

macro_rules! def_ins_primop {
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

macro_rules! def_ins_std {
    ($ins:ident) => {
        def_ins_raw!{
            $ins (m) {
                instructions::$ins(m.state, m.system)?;
                Ok(ControlFlow::Continue)
            }
        }
    }
}

macro_rules! def_ins_code {
    ($ins:ident) => {
        def_ins_raw!{
            $ins (m) {
                instructions::$ins(m.state, m.system, m.bytecode.as_ref())?;
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

        ADD: {primop}
        MUL: {primop}
        SUB: {primop}
        DIV: {primop}
        SDIV: {primop}
        MOD: {primop}
        SMOD: {primop}
        ADDMOD: {primop}
        MULMOD: {primop}
        EXP: {primop}
        SIGNEXTEND: {primop}
        LT: {primop}
        GT: {primop}
        SLT: {primop}
        SGT: {primop}
        EQ: {primop}
        ISZERO: {primop}
        AND: {primop}
        OR: {primop}
        XOR: {primop}
        NOT: {primop}
        BYTE: {primop}
        SHL: {primop}
        SHR: {primop}
        SAR: {primop}

        KECCAK256: {std}
        ADDRESS: {std}
        BALANCE: {std}
        ORIGIN: {std}
        CALLER: {std}
        CALLVALUE: {std}
        CALLDATALOAD: {std}
        CALLDATASIZE: {std}
        CALLDATACOPY: {std}

        CODESIZE: {code}
        CODECOPY: {code}

        GASPRICE: {std}
        EXTCODESIZE: {std}
        EXTCODECOPY: {std}
        RETURNDATASIZE: {std}
        RETURNDATACOPY: {std}
        EXTCODEHASH: {std}
        BLOCKHASH: {std}
        COINBASE: {std}
        TIMESTAMP: {std}
        NUMBER: {std}
        DIFFICULTY: {std}
        GASLIMIT: {std}
        CHAINID: {std}
        BASEFEE: {std}
        SELFBALANCE: {std}
        POP: {primop}
        MLOAD: {std}
        MSTORE: {std}
        MSTORE8: {std}
        SLOAD: {std}
        SSTORE: {std}

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

        MSIZE: {std}
        GAS: {std}

        JUMPDEST: {=> Ok(ControlFlow::Continue)} // noop marker opcode for valid jumps addresses

        PUSH1: {push}
        PUSH2: {push}
        PUSH3: {push}
        PUSH4: {push}
        PUSH5: {push}
        PUSH6: {push}
        PUSH7: {push}
        PUSH8: {push}
        PUSH9: {push}
        PUSH10: {push}
        PUSH11: {push}
        PUSH12: {push}
        PUSH13: {push}
        PUSH14: {push}
        PUSH15: {push}
        PUSH16: {push}
        PUSH17: {push}
        PUSH18: {push}
        PUSH19: {push}
        PUSH20: {push}
        PUSH21: {push}
        PUSH22: {push}
        PUSH23: {push}
        PUSH24: {push}
        PUSH25: {push}
        PUSH26: {push}
        PUSH27: {push}
        PUSH28: {push}
        PUSH29: {push}
        PUSH30: {push}
        PUSH31: {push}
        PUSH32: {push}

        DUP1: {primop}
        DUP2: {primop}
        DUP3: {primop}
        DUP4: {primop}
        DUP5: {primop}
        DUP6: {primop}
        DUP7: {primop}
        DUP8: {primop}
        DUP9: {primop}
        DUP10: {primop}
        DUP11: {primop}
        DUP12: {primop}
        DUP13: {primop}
        DUP14: {primop}
        DUP15: {primop}
        DUP16: {primop}

        SWAP1: {primop}
        SWAP2: {primop}
        SWAP3: {primop}
        SWAP4: {primop}
        SWAP5: {primop}
        SWAP6: {primop}
        SWAP7: {primop}
        SWAP8: {primop}
        SWAP9: {primop}
        SWAP10: {primop}
        SWAP11: {primop}
        SWAP12: {primop}
        SWAP13: {primop}
        SWAP14: {primop}
        SWAP15: {primop}
        SWAP16: {primop}

        LOG0: {std}
        LOG1: {std}
        LOG2: {std}
        LOG3: {std}
        LOG4: {std}

        CREATE: {std}

        CALL: {std}
        CALLCODE: {std}

        RETURN: {(m) => {
            instructions::control::ret(m.state)?;
            Ok(ControlFlow::Exit)
        }}

        DELEGATECALL: {std}
        CREATE2: {std}
        STATICCALL: {std}

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
