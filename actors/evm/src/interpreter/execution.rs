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
    pub system: &'r mut System<'a, RT>,
    pub state: &'r mut ExecutionState,
    pub bytecode: &'r Bytecode,
    pub pc: usize,

    // control flow
    reverted: bool,
    exit: Option<StatusCode>,
}

type Instruction<M> = fn(*mut M);

macro_rules! def_opcodes {
    ($($op:ident: $body:tt)*) => {
        def_ins_raw! {
            UNDEFINED(m) {
                m.exit = Some(StatusCode::UndefinedInstruction);
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
    ($ins:ident {intrinsic}) => {
        def_ins_intrinsic! { $ins }
    };

    ($ins:ident {=> $expr:expr}) => {
        def_ins_raw! { $ins (m) { m.exit = Some($expr); } }
    };

    ($ins:ident {($arg:ident) => $body:block}) => {
        def_ins_raw! { $ins ($arg) $body }
    };
}

macro_rules! def_ins_raw {
    ($ins:ident ($arg:ident) $body:block) => {
        #[allow(non_snake_case)]
        fn $ins(p: *mut Self) {
            // SAFETY: macro ensures that mut pointer is taken directly from a mutable borrow, used once, then goes out of scope immediately after
            let $arg: &mut Self = unsafe { p.as_mut().unwrap() };
            $body
        }
    };
}

macro_rules! def_ins_intrinsic {
    ($ins:ident) => {
        def_ins_raw! {
            $ins (m) {
                match instructions::$ins(m) {
                    Ok(_) => {
                        m.pc += 1;
                    },
                    Err(err) => {
                        m.exit = Some(err);
                    }
                }
            }
        }
    };
}

macro_rules! try_ins {
    ($ins:ident($m:ident) => ($res:ident) $body:block) => {
        match instructions::$ins($m) {
            Ok($res) => $body,
            Err(err) => {
                $m.exit = Some(err);
            }
        }
    };

    ($ins:ident($m:ident) => $body:block) => {
        match instructions::$ins($m) {
            Ok(_) => $body,
            Err(err) => {
                $m.exit = Some(err);
            }
        }
    };
}

impl<'r, 'a, RT: Runtime + 'r> Machine<'r, 'a, RT> {
    pub fn new(
        system: &'r mut System<'a, RT>,
        state: &'r mut ExecutionState,
        bytecode: &'r Bytecode,
    ) -> Self {
        Machine { system, state, bytecode, pc: 0, reverted: false, exit: None }
    }

    pub fn execute(&mut self) -> Result<(), StatusCode> {
        while self.pc <  self.bytecode.len() {
            self.step();

            if self.exit.is_some() {
                break;
            }
        }

        match &self.exit {
            None => Ok(()),
            Some(StatusCode::Success) => Ok(()),
            Some(err) => Err(err.clone()),
        }
    }

    fn step(&mut self) {
        let op = OpCode::try_from(self.bytecode[self.pc]);
        match op {
            Ok(op) => Self::JMPTABLE[op as usize](self),
            Err(err) => self.exit = Some(err),
        }
    }

    def_opcodes! {
        STOP: {=> StatusCode::Success}

        // primops
        ADD: {intrinsic}
        MUL: {intrinsic}
        SUB: {intrinsic}
        DIV: {intrinsic}
        SDIV: {intrinsic}
        MOD: {intrinsic}
        SMOD: {intrinsic}
        ADDMOD: {intrinsic}
        MULMOD: {intrinsic}
        EXP: {intrinsic}
        SIGNEXTEND: {intrinsic}
        LT: {intrinsic}
        GT: {intrinsic}
        SLT: {intrinsic}
        SGT: {intrinsic}
        EQ: {intrinsic}
        ISZERO: {intrinsic}
        AND: {intrinsic}
        OR: {intrinsic}
        XOR: {intrinsic}
        NOT: {intrinsic}
        BYTE: {intrinsic}
        SHL: {intrinsic}
        SHR: {intrinsic}
        SAR: {intrinsic}

        // std call convenction functionoids
        KECCAK256: {intrinsic}
        ADDRESS: {intrinsic}
        BALANCE: {intrinsic}
        ORIGIN: {intrinsic}
        CALLER: {intrinsic}
        CALLVALUE: {intrinsic}
        CALLDATALOAD: {intrinsic}
        CALLDATASIZE: {intrinsic}
        CALLDATACOPY: {intrinsic}
        CODESIZE: {intrinsic}
        CODECOPY: {intrinsic}
        GASPRICE: {intrinsic}
        EXTCODESIZE: {intrinsic}
        EXTCODECOPY: {intrinsic}
        RETURNDATASIZE: {intrinsic}
        RETURNDATACOPY: {intrinsic}
        EXTCODEHASH: {intrinsic}
        BLOCKHASH: {intrinsic}
        COINBASE: {intrinsic}
        TIMESTAMP: {intrinsic}
        NUMBER: {intrinsic}
        DIFFICULTY: {intrinsic}
        GASLIMIT: {intrinsic}
        CHAINID: {intrinsic}
        BASEFEE: {intrinsic}
        SELFBALANCE: {intrinsic}
        MLOAD: {intrinsic}
        MSTORE: {intrinsic}
        MSTORE8: {intrinsic}
        SLOAD: {intrinsic}
        SSTORE: {intrinsic}
        MSIZE: {intrinsic}
        GAS: {intrinsic}

        // stack ops
        POP: {intrinsic}

        // push variants
        PUSH1: {intrinsic}
        PUSH2: {intrinsic}
        PUSH3: {intrinsic}
        PUSH4: {intrinsic}
        PUSH5: {intrinsic}
        PUSH6: {intrinsic}
        PUSH7: {intrinsic}
        PUSH8: {intrinsic}
        PUSH9: {intrinsic}
        PUSH10: {intrinsic}
        PUSH11: {intrinsic}
        PUSH12: {intrinsic}
        PUSH13: {intrinsic}
        PUSH14: {intrinsic}
        PUSH15: {intrinsic}
        PUSH16: {intrinsic}
        PUSH17: {intrinsic}
        PUSH18: {intrinsic}
        PUSH19: {intrinsic}
        PUSH20: {intrinsic}
        PUSH21: {intrinsic}
        PUSH22: {intrinsic}
        PUSH23: {intrinsic}
        PUSH24: {intrinsic}
        PUSH25: {intrinsic}
        PUSH26: {intrinsic}
        PUSH27: {intrinsic}
        PUSH28: {intrinsic}
        PUSH29: {intrinsic}
        PUSH30: {intrinsic}
        PUSH31: {intrinsic}
        PUSH32: {intrinsic}

        // dup variants
        DUP1: {intrinsic}
        DUP2: {intrinsic}
        DUP3: {intrinsic}
        DUP4: {intrinsic}
        DUP5: {intrinsic}
        DUP6: {intrinsic}
        DUP7: {intrinsic}
        DUP8: {intrinsic}
        DUP9: {intrinsic}
        DUP10: {intrinsic}
        DUP11: {intrinsic}
        DUP12: {intrinsic}
        DUP13: {intrinsic}
        DUP14: {intrinsic}
        DUP15: {intrinsic}
        DUP16: {intrinsic}

        // swap variants
        SWAP1: {intrinsic}
        SWAP2: {intrinsic}
        SWAP3: {intrinsic}
        SWAP4: {intrinsic}
        SWAP5: {intrinsic}
        SWAP6: {intrinsic}
        SWAP7: {intrinsic}
        SWAP8: {intrinsic}
        SWAP9: {intrinsic}
        SWAP10: {intrinsic}
        SWAP11: {intrinsic}
        SWAP12: {intrinsic}
        SWAP13: {intrinsic}
        SWAP14: {intrinsic}
        SWAP15: {intrinsic}
        SWAP16: {intrinsic}

        // event logs
        LOG0: {intrinsic}
        LOG1: {intrinsic}
        LOG2: {intrinsic}
        LOG3: {intrinsic}
        LOG4: {intrinsic}

        // create variants
        CREATE: {intrinsic}
        CREATE2: {intrinsic}

        // call variants
        CALL: {intrinsic}
        CALLCODE: {intrinsic}
        DELEGATECALL: {intrinsic}
        STATICCALL: {intrinsic}

        // control flow magic
        // noop marker opcode for valid jumps addresses
        JUMPDEST: {(m) => {
            m.pc += 1;
        }}

        JUMP: {(m) => {
            try_ins! { JUMP(m) => (res) {
                if let Some(dest) = res {
                    m.pc = dest;
                } else {
                    // cant happen, unless it's a cosmic ray
                    m.exit = Some(StatusCode::Failure);
                }
            }}
        }}

        JUMPI: {(m) => {
            try_ins! { JUMPI(m) => (res) {
                if let Some(dest) = res {
                    m.pc = dest;
                } else {
                    m.pc += 1;
                }
            }}
        }}

        PC: {(m) => {
            try_ins! { PC(m) => {
                m.pc += 1;
            }}
        }}

        RETURN: {(m) => {
            try_ins! { RETURN(m) => {
                m.exit = Some(StatusCode::Success);
            }}
        }}

        REVERT: {(m) => {
            try_ins! { REVERT(m) => {
                m.reverted = true;
                m.exit = Some(StatusCode::Success);
            }}
        }}

        SELFDESTRUCT: {(m) => {
            try_ins! { SELFDESTRUCT(m) => {
                m.exit = Some(StatusCode::Success);
            }}
        }}

        INVALID: {=> StatusCode::InvalidInstruction}
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
