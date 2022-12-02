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
    pub output: Output,
}

type Instruction<M> = unsafe fn(*mut M) -> Result<(), StatusCode>;

macro_rules! def_opcodes {
    ($($op:ident)*) => {
        def_ins_raw! {
            UNDEFINED(_m) {
                Err(StatusCode::UndefinedInstruction)
            }
        }
        $(def_ins_raw! {
            $op (m) {
                instructions::$op(m)
            }
        })*

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

macro_rules! def_ins_raw {
    ($ins:ident ($arg:ident) $body:block) => {
        #[allow(non_snake_case)]
        unsafe fn $ins(p: *mut Self) -> Result<(), StatusCode> {
            // SAFETY: macro ensures that mut pointer is taken directly from a mutable borrow, used
            // once, then goes out of scope immediately after
            let $arg: &mut Self = &mut *p;
            $body
        }
    };
}

impl<'r, 'a, RT: Runtime + 'r> Machine<'r, 'a, RT> {
    pub fn new(
        system: &'r mut System<'a, RT>,
        state: &'r mut ExecutionState,
        bytecode: &'r Bytecode,
    ) -> Self {
        Machine { system, state, bytecode, pc: 0, output: Output::default() }
    }

    pub fn execute(mut self) -> Result<Output, StatusCode> {
        while self.pc < self.bytecode.len() {
            // This is faster than the question mark operator, and speed counts here.
            if let Err(e) = self.step() {
                return Err(e);
            }
        }

        Ok(self.output)
    }

    #[inline(always)]
    fn step(&mut self) -> Result<(), StatusCode> {
        let op = self.bytecode[self.pc];
        unsafe { Self::JMPTABLE[op as usize](self) }
    }

    def_opcodes! {
        // primops
        ADD
        MUL
        SUB
        DIV
        SDIV
        MOD
        SMOD
        ADDMOD
        MULMOD
        EXP
        SIGNEXTEND
        LT
        GT
        SLT
        SGT
        EQ
        ISZERO
        AND
        OR
        XOR
        NOT
        BYTE
        SHL
        SHR
        SAR

        // std call convenction functionoids
        KECCAK256
        ADDRESS
        BALANCE
        ORIGIN
        CALLER
        CALLVALUE
        CALLDATALOAD
        CALLDATASIZE
        CALLDATACOPY
        CODESIZE
        CODECOPY
        GASPRICE
        EXTCODESIZE
        EXTCODECOPY
        RETURNDATASIZE
        RETURNDATACOPY
        EXTCODEHASH
        BLOCKHASH
        COINBASE
        TIMESTAMP
        NUMBER
        DIFFICULTY
        GASLIMIT
        CHAINID
        BASEFEE
        SELFBALANCE
        MLOAD
        MSTORE
        MSTORE8
        SLOAD
        SSTORE
        MSIZE
        GAS

        // stack ops
        POP

        // push variants
        PUSH1
        PUSH2
        PUSH3
        PUSH4
        PUSH5
        PUSH6
        PUSH7
        PUSH8
        PUSH9
        PUSH10
        PUSH11
        PUSH12
        PUSH13
        PUSH14
        PUSH15
        PUSH16
        PUSH17
        PUSH18
        PUSH19
        PUSH20
        PUSH21
        PUSH22
        PUSH23
        PUSH24
        PUSH25
        PUSH26
        PUSH27
        PUSH28
        PUSH29
        PUSH30
        PUSH31
        PUSH32

        // dup variants
        DUP1
        DUP2
        DUP3
        DUP4
        DUP5
        DUP6
        DUP7
        DUP8
        DUP9
        DUP10
        DUP11
        DUP12
        DUP13
        DUP14
        DUP15
        DUP16

        // swap variants
        SWAP1
        SWAP2
        SWAP3
        SWAP4
        SWAP5
        SWAP6
        SWAP7
        SWAP8
        SWAP9
        SWAP10
        SWAP11
        SWAP12
        SWAP13
        SWAP14
        SWAP15
        SWAP16

        // pc
        PC

        // event logs
        LOG0
        LOG1
        LOG2
        LOG3
        LOG4

        // create variants
        CREATE
        CREATE2

        // call variants
        CALL
        CALLCODE
        DELEGATECALL
        STATICCALL

        // exiting ops
        STOP
        RETURN
        REVERT
        SELFDESTRUCT

        // control flow magic
        // noop marker opcode for valid jumps addresses
        JUMPDEST
        // reserved invalid instruction
        INVALID

        // jump ops
        JUMP
        JUMPI
    }

    const JMPTABLE: [Instruction<Machine<'r, 'a, RT>>; 256] = Machine::<'r, 'a, RT>::jmptable();
}

pub fn execute(
    bytecode: &Bytecode,
    runtime: &mut ExecutionState,
    system: &mut System<impl Runtime>,
) -> Result<Output, StatusCode> {
    Machine::new(system, runtime, bytecode).execute()
}
