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
    ($ins:ident {"primop"}) => {
        def_ins_primop! { $ins }
    };

    ($ins:ident {($arg:ident) $body:block}) => {
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
        STOP: {(_m) {
            Ok(ControlFlow::Exit)
        }}

        ADD: {"primop"}
        MUL: {"primop"}
        SUB: {"primop"}
        DIV: {"primop"}
        SDIV: {"primop"}
        MOD: {"primop"}
        SMOD: {"primop"}
        ADDMOD: {"primop"}
        MULMOD: {"primop"}
        EXP: {"primop"}
        SIGNEXTEND: {"primop"}
        LT: {"primop"}
        GT: {"primop"}
        SLT: {"primop"}
        SGT: {"primop"}
        EQ: {"primop"}
        ISZERO: {"primop"}
        AND: {"primop"}
        OR: {"primop"}
        XOR: {"primop"}
        NOT: {"primop"}
        BYTE: {"primop"}
        SHL: {"primop"}
        SHR: {"primop"}
        SAR: {"primop"}

        KECCAK256: {(m) {
            instructions::hash::keccak256(m.system, m.state)?;
            Ok(ControlFlow::Continue)
        }}

        ADDRESS: {(m) {
            instructions::context::address(m.state, m.system);
            Ok(ControlFlow::Continue)
        }}

        BALANCE: {(m) {
            instructions::state::balance(m.state, m.system)?;
            Ok(ControlFlow::Continue)
        }}

        ORIGIN: {(m) {
            instructions::context::origin(m.state, m.system);
            Ok(ControlFlow::Continue)
        }}

        CALLER: {(m) {
            instructions::context::caller(m.state, m.system);
            Ok(ControlFlow::Continue)
        }}

        CALLVALUE: {(m) {
            instructions::context::call_value(m.state, m.system);
            Ok(ControlFlow::Continue)
        }}

        CALLDATALOAD: {(m) {
            instructions::call::calldataload(m.state);
            Ok(ControlFlow::Continue)
        }}

        CALLDATASIZE: {(m) {
            instructions::call::calldatasize(m.state);
            Ok(ControlFlow::Continue)
        }}

        CALLDATACOPY: {(m) {
            instructions::call::calldatacopy(m.state)?;
            Ok(ControlFlow::Continue)
        }}

        CODESIZE: {(m) {
            instructions::call::codesize(&mut m.state.stack, m.bytecode.as_ref());
            Ok(ControlFlow::Continue)
        }}

        CODECOPY: {(m) {
            instructions::call::codecopy(m.state, m.bytecode.as_ref())?;
            Ok(ControlFlow::Continue)
        }}

        GASPRICE: {(m) {
            instructions::context::gas_price(m.state, m.system);
            Ok(ControlFlow::Continue)
        }}

        EXTCODESIZE: {(m) {
            instructions::ext::extcodesize(m.state, m.system)?;
            Ok(ControlFlow::Continue)
        }}

        EXTCODECOPY: {(m) {
            instructions::ext::extcodecopy(m.state, m.system)?;
            Ok(ControlFlow::Continue)
        }}

        RETURNDATASIZE: {(m) {
            instructions::control::returndatasize(m.state);
            Ok(ControlFlow::Continue)
        }}

        RETURNDATACOPY: {(m) {
            instructions::control::returndatacopy(m.state)?;
            Ok(ControlFlow::Continue)
        }}

        EXTCODEHASH: {(m) {
            instructions::ext::extcodehash(m.state, m.system)?;
            Ok(ControlFlow::Continue)
        }}

        BLOCKHASH: {(m) {
            instructions::context::blockhash(m.state, m.system);
            Ok(ControlFlow::Continue)
        }}

        COINBASE: {(m) {
            instructions::context::coinbase(m.state, m.system);
            Ok(ControlFlow::Continue)
        }}

        TIMESTAMP: {(m) {
            instructions::context::timestamp(m.state, m.system);
            Ok(ControlFlow::Continue)
        }}

        NUMBER: {(m) {
            instructions::context::block_number(m.state, m.system);
            Ok(ControlFlow::Continue)
        }}

        DIFFICULTY: {(m) {
            instructions::context::difficulty(m.state, m.system);
            Ok(ControlFlow::Continue)
        }}

        GASLIMIT: {(m) {
            instructions::context::gas_limit(m.state, m.system);
            Ok(ControlFlow::Continue)
        }}

        CHAINID: {(m) {
            instructions::context::chain_id(m.state, m.system);
            Ok(ControlFlow::Continue)
        }}

        SELFBALANCE: {(m) {
            instructions::state::selfbalance(m.state, m.system);
            Ok(ControlFlow::Continue)
        }}

        BASEFEE: {(m) {
            instructions::context::base_fee(m.state, m.system);
            Ok(ControlFlow::Continue)
        }}

        POP: {(m) {
            instructions::stack::pop(&mut m.state.stack);
            Ok(ControlFlow::Continue)
        }}

        MLOAD: {(m) {
            instructions::memory::mload(m.state)?;
            Ok(ControlFlow::Continue)
        }}

        MSTORE: {(m) {
            instructions::memory::mstore(m.state)?;
            Ok(ControlFlow::Continue)
        }}

        MSTORE8: {(m) {
            instructions::memory::mstore8(m.state)?;
            Ok(ControlFlow::Continue)
        }}

        SLOAD: {(m) {
            instructions::storage::sload(m.state, m.system)?;
            Ok(ControlFlow::Continue)
        }}

        SSTORE: {(m) {
            instructions::storage::sstore(m.state, m.system)?;
            Ok(ControlFlow::Continue)
        }}

        JUMP: {(m) {
            m.pc = instructions::control::jump(&mut m.state.stack, m.bytecode)?;
            Ok(ControlFlow::Jump)
        }}

        JUMPI: {(m) {
            if let Some(dest) = instructions::control::jumpi(&mut m.state.stack, m.bytecode)? {
                m.pc = dest;
                Ok(ControlFlow::Jump)
            } else {
                Ok(ControlFlow::Continue)
            }
        }}

        PC: {(m) {
            instructions::control::pc(&mut m.state.stack, m.pc);
            Ok(ControlFlow::Continue)
        }}

        MSIZE: {(m) {
            instructions::memory::msize(m.state);
            Ok(ControlFlow::Continue)
        }}

        GAS: {(m) {
            instructions::context::gas(m.state, m.system);
            Ok(ControlFlow::Continue)
        }}

        JUMPDEST: {(_m) {
            // marker opcode for valid jumps addresses
            Ok(ControlFlow::Continue)
        }}

        PUSH1: {(m) {
            m.pc += instructions::stack::push::<1>(&mut m.state.stack, &m.bytecode[m.pc + 1..]);
            Ok(ControlFlow::Continue)
        }}

        PUSH2: {(m) {
            m.pc += instructions::stack::push::<2>(&mut m.state.stack, &m.bytecode[m.pc + 1..]);
            Ok(ControlFlow::Continue)
        }}

        PUSH3: {(m) {
            m.pc += instructions::stack::push::<3>(&mut m.state.stack, &m.bytecode[m.pc + 1..]);
            Ok(ControlFlow::Continue)
        }}

        PUSH4: {(m) {
            m.pc += instructions::stack::push::<4>(&mut m.state.stack, &m.bytecode[m.pc + 1..]);
            Ok(ControlFlow::Continue)
        }}

        PUSH5: {(m) {
            m.pc += instructions::stack::push::<5>(&mut m.state.stack, &m.bytecode[m.pc + 1..]);
            Ok(ControlFlow::Continue)
        }}

        PUSH6: {(m) {
            m.pc += instructions::stack::push::<6>(&mut m.state.stack, &m.bytecode[m.pc + 1..]);
            Ok(ControlFlow::Continue)
        }}

        PUSH7: {(m) {
            m.pc += instructions::stack::push::<7>(&mut m.state.stack, &m.bytecode[m.pc + 1..]);
            Ok(ControlFlow::Continue)
        }}

        PUSH8: {(m) {
            m.pc += instructions::stack::push::<8>(&mut m.state.stack, &m.bytecode[m.pc + 1..]);
            Ok(ControlFlow::Continue)
        }}

        PUSH9: {(m) {
            m.pc += instructions::stack::push::<9>(&mut m.state.stack, &m.bytecode[m.pc + 1..]);
            Ok(ControlFlow::Continue)
        }}

        PUSH10: {(m) {
            m.pc += instructions::stack::push::<10>(&mut m.state.stack, &m.bytecode[m.pc + 1..]);
            Ok(ControlFlow::Continue)
        }}

        PUSH11: {(m) {
            m.pc += instructions::stack::push::<11>(&mut m.state.stack, &m.bytecode[m.pc + 1..]);
            Ok(ControlFlow::Continue)
        }}

        PUSH12: {(m) {
            m.pc += instructions::stack::push::<12>(&mut m.state.stack, &m.bytecode[m.pc + 1..]);
            Ok(ControlFlow::Continue)
        }}

        PUSH13: {(m) {
            m.pc += instructions::stack::push::<13>(&mut m.state.stack, &m.bytecode[m.pc + 1..]);
            Ok(ControlFlow::Continue)
        }}

        PUSH14: {(m) {
            m.pc += instructions::stack::push::<14>(&mut m.state.stack, &m.bytecode[m.pc + 1..]);
            Ok(ControlFlow::Continue)
        }}

        PUSH15: {(m) {
            m.pc += instructions::stack::push::<15>(&mut m.state.stack, &m.bytecode[m.pc + 1..]);
            Ok(ControlFlow::Continue)
        }}

        PUSH16: {(m) {
            m.pc += instructions::stack::push::<16>(&mut m.state.stack, &m.bytecode[m.pc + 1..]);
            Ok(ControlFlow::Continue)
        }}

        PUSH17: {(m) {
            m.pc += instructions::stack::push::<17>(&mut m.state.stack, &m.bytecode[m.pc + 1..]);
            Ok(ControlFlow::Continue)
        }}

        PUSH18: {(m) {
            m.pc += instructions::stack::push::<18>(&mut m.state.stack, &m.bytecode[m.pc + 1..]);
            Ok(ControlFlow::Continue)
        }}

        PUSH19: {(m) {
            m.pc += instructions::stack::push::<19>(&mut m.state.stack, &m.bytecode[m.pc + 1..]);
            Ok(ControlFlow::Continue)
        }}

        PUSH20: {(m) {
            m.pc += instructions::stack::push::<20>(&mut m.state.stack, &m.bytecode[m.pc + 1..]);
            Ok(ControlFlow::Continue)
        }}

        PUSH21: {(m) {
            m.pc += instructions::stack::push::<21>(&mut m.state.stack, &m.bytecode[m.pc + 1..]);
            Ok(ControlFlow::Continue)
        }}

        PUSH22: {(m) {
            m.pc += instructions::stack::push::<22>(&mut m.state.stack, &m.bytecode[m.pc + 1..]);
            Ok(ControlFlow::Continue)
        }}

        PUSH23: {(m) {
            m.pc += instructions::stack::push::<23>(&mut m.state.stack, &m.bytecode[m.pc + 1..]);
            Ok(ControlFlow::Continue)
        }}

        PUSH24: {(m) {
            m.pc += instructions::stack::push::<24>(&mut m.state.stack, &m.bytecode[m.pc + 1..]);
            Ok(ControlFlow::Continue)
        }}

        PUSH25: {(m) {
            m.pc += instructions::stack::push::<25>(&mut m.state.stack, &m.bytecode[m.pc + 1..]);
            Ok(ControlFlow::Continue)
        }}

        PUSH26: {(m) {
            m.pc += instructions::stack::push::<26>(&mut m.state.stack, &m.bytecode[m.pc + 1..]);
            Ok(ControlFlow::Continue)
        }}

        PUSH27: {(m) {
            m.pc += instructions::stack::push::<27>(&mut m.state.stack, &m.bytecode[m.pc + 1..]);
            Ok(ControlFlow::Continue)
        }}

        PUSH28: {(m) {
            m.pc += instructions::stack::push::<28>(&mut m.state.stack, &m.bytecode[m.pc + 1..]);
            Ok(ControlFlow::Continue)
        }}

        PUSH29: {(m) {
            m.pc += instructions::stack::push::<29>(&mut m.state.stack, &m.bytecode[m.pc + 1..]);
            Ok(ControlFlow::Continue)
        }}

        PUSH30: {(m) {
            m.pc += instructions::stack::push::<30>(&mut m.state.stack, &m.bytecode[m.pc + 1..]);
            Ok(ControlFlow::Continue)
        }}

        PUSH31: {(m) {
            m.pc += instructions::stack::push::<31>(&mut m.state.stack, &m.bytecode[m.pc + 1..]);
            Ok(ControlFlow::Continue)
        }}

        PUSH32: {(m) {
            m.pc += instructions::stack::push::<32>(&mut m.state.stack, &m.bytecode[m.pc + 1..]);
            Ok(ControlFlow::Continue)
        }}

        DUP1: {(m) {
            instructions::stack::dup::<1>(&mut m.state.stack);
            Ok(ControlFlow::Continue)
        }}

        DUP2: {(m) {
            instructions::stack::dup::<2>(&mut m.state.stack);
            Ok(ControlFlow::Continue)
        }}

        DUP3: {(m) {
            instructions::stack::dup::<3>(&mut m.state.stack);
            Ok(ControlFlow::Continue)
        }}

        DUP4: {(m) {
            instructions::stack::dup::<4>(&mut m.state.stack);
            Ok(ControlFlow::Continue)
        }}

        DUP5: {(m) {
            instructions::stack::dup::<5>(&mut m.state.stack);
            Ok(ControlFlow::Continue)
        }}

        DUP6: {(m) {
            instructions::stack::dup::<6>(&mut m.state.stack);
            Ok(ControlFlow::Continue)
        }}

        DUP7: {(m) {
            instructions::stack::dup::<7>(&mut m.state.stack);
            Ok(ControlFlow::Continue)
        }}

        DUP8: {(m) {
            instructions::stack::dup::<8>(&mut m.state.stack);
            Ok(ControlFlow::Continue)
        }}

        DUP9: {(m) {
            instructions::stack::dup::<9>(&mut m.state.stack);
            Ok(ControlFlow::Continue)
        }}

        DUP10: {(m) {
            instructions::stack::dup::<10>(&mut m.state.stack);
            Ok(ControlFlow::Continue)
        }}

        DUP11: {(m) {
            instructions::stack::dup::<11>(&mut m.state.stack);
            Ok(ControlFlow::Continue)
        }}

        DUP12: {(m) {
            instructions::stack::dup::<12>(&mut m.state.stack);
            Ok(ControlFlow::Continue)
        }}

        DUP13: {(m) {
            instructions::stack::dup::<13>(&mut m.state.stack);
            Ok(ControlFlow::Continue)
        }}

        DUP14: {(m) {
            instructions::stack::dup::<14>(&mut m.state.stack);
            Ok(ControlFlow::Continue)
        }}

        DUP15: {(m) {
            instructions::stack::dup::<15>(&mut m.state.stack);
            Ok(ControlFlow::Continue)
        }}

        DUP16: {(m) {
            instructions::stack::dup::<16>(&mut m.state.stack);
            Ok(ControlFlow::Continue)
        }}

        SWAP1: {(m) {
            instructions::stack::swap::<1>(&mut m.state.stack);
            Ok(ControlFlow::Continue)
        }}

        SWAP2: {(m) {
            instructions::stack::swap::<2>(&mut m.state.stack);
            Ok(ControlFlow::Continue)
        }}

        SWAP3: {(m) {
            instructions::stack::swap::<3>(&mut m.state.stack);
            Ok(ControlFlow::Continue)
        }}

        SWAP4: {(m) {
            instructions::stack::swap::<4>(&mut m.state.stack);
            Ok(ControlFlow::Continue)
        }}

        SWAP5: {(m) {
            instructions::stack::swap::<5>(&mut m.state.stack);
            Ok(ControlFlow::Continue)
        }}

        SWAP6: {(m) {
            instructions::stack::swap::<6>(&mut m.state.stack);
            Ok(ControlFlow::Continue)
        }}

        SWAP7: {(m) {
            instructions::stack::swap::<7>(&mut m.state.stack);
            Ok(ControlFlow::Continue)
        }}

        SWAP8: {(m) {
            instructions::stack::swap::<8>(&mut m.state.stack);
            Ok(ControlFlow::Continue)
        }}

        SWAP9: {(m) {
            instructions::stack::swap::<9>(&mut m.state.stack);
            Ok(ControlFlow::Continue)
        }}

        SWAP10: {(m) {
            instructions::stack::swap::<10>(&mut m.state.stack);
            Ok(ControlFlow::Continue)
        }}

        SWAP11: {(m) {
            instructions::stack::swap::<11>(&mut m.state.stack);
            Ok(ControlFlow::Continue)
        }}

        SWAP12: {(m) {
            instructions::stack::swap::<12>(&mut m.state.stack);
            Ok(ControlFlow::Continue)
        }}

        SWAP13: {(m) {
            instructions::stack::swap::<13>(&mut m.state.stack);
            Ok(ControlFlow::Continue)
        }}

        SWAP14: {(m) {
            instructions::stack::swap::<14>(&mut m.state.stack);
            Ok(ControlFlow::Continue)
        }}

        SWAP15: {(m) {
            instructions::stack::swap::<15>(&mut m.state.stack);
            Ok(ControlFlow::Continue)
        }}

        SWAP16: {(m) {
            instructions::stack::swap::<16>(&mut m.state.stack);
            Ok(ControlFlow::Continue)
        }}

        LOG0: {(m) {
            instructions::log::log(m.state, m.system, 0)?;
            Ok(ControlFlow::Continue)
        }}

        LOG1: {(m) {
            instructions::log::log(m.state, m.system, 1)?;
            Ok(ControlFlow::Continue)
        }}

        LOG2: {(m) {
            instructions::log::log(m.state, m.system, 2)?;
            Ok(ControlFlow::Continue)
        }}

        LOG3: {(m) {
            instructions::log::log(m.state, m.system, 3)?;
            Ok(ControlFlow::Continue)
        }}

        LOG4: {(m) {
            instructions::log::log(m.state, m.system, 4)?;
            Ok(ControlFlow::Continue)
        }}

        CREATE: {(m) {
            instructions::lifecycle::create(m.state, m.system)?;
            Ok(ControlFlow::Continue)
        }}

        CALL: {(m) {
            instructions::call::call(m.state, m.system, CallKind::Call)?;
            Ok(ControlFlow::Continue)
        }}

        CALLCODE: {(m) {
            instructions::call::call(m.state, m.system, CallKind::CallCode)?;
            Ok(ControlFlow::Continue)
        }}

        RETURN: {(m) {
            instructions::control::ret(m.state)?;
            Ok(ControlFlow::Exit)
        }}

        DELEGATECALL: {(m) {
            instructions::call::call(m.state, m.system, CallKind::DelegateCall)?;
            Ok(ControlFlow::Continue)
        }}

        CREATE2: {(m) {
            instructions::lifecycle::create2(m.state, m.system)?;
            Ok(ControlFlow::Continue)
        }}

        STATICCALL: {(m) {
            instructions::call::call(m.state, m.system, CallKind::StaticCall)?;
            Ok(ControlFlow::Continue)
        }}

        REVERT: {(m) {
            instructions::control::ret(m.state)?;
            m.reverted = true;
            Ok(ControlFlow::Exit)
        }}

        INVALID: {(_m) {
            Err(StatusCode::InvalidInstruction)
        }}

        SELFDESTRUCT: {(m) {
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
