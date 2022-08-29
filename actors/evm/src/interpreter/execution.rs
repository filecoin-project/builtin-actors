#![allow(dead_code)]

use {
    super::instructions::*,
    super::opcode::OpCode,
    super::CallKind,
    super::StatusCode,
    crate::interpreter::instructions::log::*,
    crate::interpreter::instructions::stack::*,
    crate::interpreter::memory::Memory,
    crate::interpreter::stack::Stack,
    crate::interpreter::{Bytecode, Output, System},
    bytes::Bytes,
    fil_actors_runtime::runtime::Runtime,
    fvm_ipld_blockstore::Blockstore,
};

/// EVM execution runtime.
#[derive(Clone, Debug)]
pub struct ExecutionState {
    pub stack: Stack,
    pub memory: Memory,
    pub input_data: Bytes,
    pub return_data: Bytes,
    pub output_data: Bytes,
}

impl ExecutionState {
    pub fn new(input_data: Bytes) -> Self {
        Self {
            stack: Stack::default(),
            memory: Memory::default(),
            input_data,
            return_data: Default::default(),
            output_data: Bytes::new(),
        }
    }
}

struct Machine<'r, BS: Blockstore, RT: Runtime<BS>> {
    system: &'r System<'r, BS, RT>,
    runtime: &'r mut ExecutionState,
    bytecode: &'r Bytecode<'r>,
    pc: usize,
    reverted: bool,
}

enum ControlFlow {
    Continue,
    Jump,
    Exit,
}

type Instruction<M> = fn(*mut M) -> Result<ControlFlow, StatusCode>;

macro_rules! def_jmptable {
    ($($op:ident)*) => {
        const fn jmptable() -> [Instruction<Machine<'r, BS, RT>>; 256] {
            let mut table: [Instruction<Machine::<'r, BS, RT>>; 256] = [Machine::<'r, BS, RT>::UNDEFINED; 256];
            $(table[OpCode::$op as usize] = Machine::<'r, BS, RT>::$op;)*
            table
        }
    }
}

macro_rules! def_ins1 {
    ($ins:ident ($arg:ident) $body:block) => {
        #[allow(non_snake_case)]
        fn $ins(p: *mut Self) -> Result<ControlFlow, StatusCode> {
            // SAFETY: macro ensures that mut pointer is taken directly from a mutable borrow, used once, then goes out of scope immediately after
            let $arg: &mut Self = unsafe { p.as_mut().unwrap() };
            $body
        }
    };
}

macro_rules! def_ins {
    ($($op:ident  ($arg:ident) $body:block)*) => {
        def_ins1! {
            UNDEFINED(_m) {
                Err(StatusCode::UndefinedInstruction)
            }
        }
        $(def_ins1! { $op ($arg) $body })*
        def_jmptable! {
            $($op)*
        }
    }
}

impl<'r, BS: Blockstore + 'r, RT: Runtime<BS> + 'r> Machine<'r, BS, RT> {
    pub fn new(
        system: &'r System<'r, BS, RT>,
        runtime: &'r mut ExecutionState,
        bytecode: &'r Bytecode,
    ) -> Self {
        Machine { system, runtime, bytecode, pc: 0, reverted: false }
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
        Machine::<'r, BS, RT>::dispatch(op)(self)
    }

    // Beware, dragons!
    fn dispatch(op: OpCode) -> Instruction<Machine<'r, BS, RT>> {
        Self::JMPTABLE[op as usize]
    }

    def_ins! {
        STOP(_m) {
            Ok(ControlFlow::Exit)
        }

        ADD(m) {
            arithmetic::add(&mut m.runtime.stack);
            Ok(ControlFlow::Continue)
        }

        MUL(m) {
            arithmetic::mul(&mut m.runtime.stack);
            Ok(ControlFlow::Continue)
        }

        SUB(m) {
            arithmetic::sub(&mut m.runtime.stack);
            Ok(ControlFlow::Continue)
        }

        DIV(m) {
            arithmetic::div(&mut m.runtime.stack);
            Ok(ControlFlow::Continue)
        }

        SDIV(m) {
            arithmetic::sdiv(&mut m.runtime.stack);
            Ok(ControlFlow::Continue)
        }

        MOD(m) {
            arithmetic::modulo(&mut m.runtime.stack);
            Ok(ControlFlow::Continue)
        }

        SMOD(m) {
            arithmetic::smod(&mut m.runtime.stack);
            Ok(ControlFlow::Continue)
        }

        ADDMOD(m) {
            arithmetic::addmod(&mut m.runtime.stack);
            Ok(ControlFlow::Continue)
        }

        MULMOD(m) {
            arithmetic::mulmod(&mut m.runtime.stack);
            Ok(ControlFlow::Continue)
        }

        EXP(m) {
            arithmetic::exp(&mut m.runtime.stack);
            Ok(ControlFlow::Continue)
        }

        SIGNEXTEND(m) {
            arithmetic::signextend(&mut m.runtime.stack);
            Ok(ControlFlow::Continue)
        }

        LT(m) {
            boolean::lt(&mut m.runtime.stack);
            Ok(ControlFlow::Continue)
        }

        GT(m) {
            boolean::gt(&mut m.runtime.stack);
            Ok(ControlFlow::Continue)
        }

        SLT(m) {
            boolean::slt(&mut m.runtime.stack);
            Ok(ControlFlow::Continue)
        }

        SGT(m) {
            boolean::sgt(&mut m.runtime.stack);
            Ok(ControlFlow::Continue)
        }

        EQ(m) {
            boolean::eq(&mut m.runtime.stack);
            Ok(ControlFlow::Continue)
        }

        ISZERO(m) {
            boolean::iszero(&mut m.runtime.stack);
            Ok(ControlFlow::Continue)
        }

        AND(m) {
            boolean::and(&mut m.runtime.stack);
            Ok(ControlFlow::Continue)
        }

        OR(m) {
            boolean::or(&mut m.runtime.stack);
            Ok(ControlFlow::Continue)
        }

        XOR(m) {
            boolean::xor(&mut m.runtime.stack);
            Ok(ControlFlow::Continue)
        }

        NOT(m) {
            boolean::not(&mut m.runtime.stack);
            Ok(ControlFlow::Continue)
        }

        BYTE(m) {
            bitwise::byte(&mut m.runtime.stack);
            Ok(ControlFlow::Continue)
        }

        SHL(m) {
            bitwise::shl(&mut m.runtime.stack);
            Ok(ControlFlow::Continue)
        }

        SHR(m) {
            bitwise::shr(&mut m.runtime.stack);
            Ok(ControlFlow::Continue)
        }

        SAR(m) {
            bitwise::sar(&mut m.runtime.stack);
            Ok(ControlFlow::Continue)
        }

        KECCAK256(m) {
            hash::keccak256(m.runtime)?;
            Ok(ControlFlow::Continue)
        }

        ADDRESS(m) {
            context::address(m.runtime, m.system);
            Ok(ControlFlow::Continue)
        }

        BALANCE(m) {
            state::balance(m.runtime, m.system)?;
            Ok(ControlFlow::Continue)
        }

        ORIGIN(m) {
            context::origin(m.runtime, m.system);
            Ok(ControlFlow::Continue)
        }

        CALLER(m) {
            context::caller(m.runtime, m.system);
            Ok(ControlFlow::Continue)
        }

        CALLVALUE(m) {
            context::call_value(m.runtime, m.system);
            Ok(ControlFlow::Continue)
        }

        CALLDATALOAD(m) {
            call::calldataload(m.runtime);
            Ok(ControlFlow::Continue)
        }

        CALLDATASIZE(m) {
            call::calldatasize(m.runtime);
            Ok(ControlFlow::Continue)
        }

        CALLDATACOPY(m) {
            call::calldatacopy(m.runtime)?;
            Ok(ControlFlow::Continue)
        }

        CODESIZE(m) {
            call::codesize(&mut m.runtime.stack, m.bytecode.as_ref());
            Ok(ControlFlow::Continue)
        }

        CODECOPY(m) {
            call::codecopy(m.runtime, m.bytecode.as_ref())?;
            Ok(ControlFlow::Continue)
        }

        GASPRICE(m) {
            context::gas_price(m.runtime, m.system)?;
            Ok(ControlFlow::Continue)
        }

        EXTCODESIZE(m) {
            ext::extcodesize(m.runtime, m.system)?;
            Ok(ControlFlow::Continue)
        }

        EXTCODECOPY(m) {
            ext::extcodecopy(m.runtime, m.system)?;
            Ok(ControlFlow::Continue)
        }

        RETURNDATASIZE(m) {
            control::returndatasize(m.runtime);
            Ok(ControlFlow::Continue)
        }

        RETURNDATACOPY(m) {
            control::returndatacopy(m.runtime)?;
            Ok(ControlFlow::Continue)
        }

        EXTCODEHASH(m) {
            ext::extcodehash(m.runtime, m.system)?;
            Ok(ControlFlow::Continue)
        }

        BLOCKHASH(m) {
            context::blockhash(m.runtime, m.system)?;
            Ok(ControlFlow::Continue)
        }

        COINBASE(m) {
            context::coinbase(m.runtime, m.system)?;
            Ok(ControlFlow::Continue)
        }

        TIMESTAMP(m) {
            context::timestamp(m.runtime, m.system)?;
            Ok(ControlFlow::Continue)
        }

        NUMBER(m) {
            context::block_number(m.runtime, m.system)?;
            Ok(ControlFlow::Continue)
        }

        DIFFICULTY(m) {
            context::difficulty(m.runtime, m.system)?;
            Ok(ControlFlow::Continue)
        }

        GASLIMIT(m) {
            context::gas_limit(m.runtime, m.system)?;
            Ok(ControlFlow::Continue)
        }

        CHAINID(m) {
            context::chain_id(m.runtime, m.system)?;
            Ok(ControlFlow::Continue)
        }

        SELFBALANCE(m) {
            state::selfbalance(m.runtime, m.system);
            Ok(ControlFlow::Continue)
        }

        BASEFEE(m) {
            context::base_fee(m.runtime, m.system);
            Ok(ControlFlow::Continue)
        }

        POP(m) {
            stack::pop(&mut m.runtime.stack);
            Ok(ControlFlow::Continue)
        }

        MLOAD(m) {
            memory::mload(m.runtime)?;
            Ok(ControlFlow::Continue)
        }

        MSTORE(m) {
            memory::mstore(m.runtime)?;
            Ok(ControlFlow::Continue)
        }

        MSTORE8(m) {
            memory::mstore8(m.runtime)?;
            Ok(ControlFlow::Continue)
        }

        SLOAD(m) {
            storage::sload(m.runtime, m.system)?;
            Ok(ControlFlow::Continue)
        }

        SSTORE(m) {
            storage::sstore(m.runtime, m.system)?;
            Ok(ControlFlow::Continue)
        }

        JUMP(m) {
            m.pc = control::jump(&mut m.runtime.stack, m.bytecode)?;
            Ok(ControlFlow::Jump)
        }

        JUMPI(m) {
            if let Some(dest) = control::jumpi(&mut m.runtime.stack, m.bytecode)? {
                m.pc = dest;
                Ok(ControlFlow::Jump)
            } else {
                Ok(ControlFlow::Continue)
            }
        }

        PC(m) {
            control::pc(&mut m.runtime.stack, m.pc);
            Ok(ControlFlow::Continue)
        }

        MSIZE(m) {
            memory::msize(m.runtime);
            Ok(ControlFlow::Continue)
        }

        GAS(m) {
            control::gas(m.runtime);
            Ok(ControlFlow::Continue)
        }

        JUMPDEST(_m) {
            // marker opcode for valid jumps addresses
            Ok(ControlFlow::Continue)
        }

        PUSH1(m) {
            m.pc += push::<1>(&mut m.runtime.stack, &m.bytecode[m.pc + 1..]);
            Ok(ControlFlow::Continue)
        }

        PUSH2(m) {
            m.pc += push::<2>(&mut m.runtime.stack, &m.bytecode[m.pc + 1..]);
            Ok(ControlFlow::Continue)
        }

        PUSH3(m) {
            m.pc += push::<3>(&mut m.runtime.stack, &m.bytecode[m.pc + 1..]);
            Ok(ControlFlow::Continue)
        }

        PUSH4(m) {
            m.pc += push::<4>(&mut m.runtime.stack, &m.bytecode[m.pc + 1..]);
            Ok(ControlFlow::Continue)
        }

        PUSH5(m) {
            m.pc += push::<5>(&mut m.runtime.stack, &m.bytecode[m.pc + 1..]);
            Ok(ControlFlow::Continue)
        }

        PUSH6(m) {
            m.pc += push::<6>(&mut m.runtime.stack, &m.bytecode[m.pc + 1..]);
            Ok(ControlFlow::Continue)
        }

        PUSH7(m) {
            m.pc += push::<7>(&mut m.runtime.stack, &m.bytecode[m.pc + 1..]);
            Ok(ControlFlow::Continue)
        }

        PUSH8(m) {
            m.pc += push::<8>(&mut m.runtime.stack, &m.bytecode[m.pc + 1..]);
            Ok(ControlFlow::Continue)
        }

        PUSH9(m) {
            m.pc += push::<9>(&mut m.runtime.stack, &m.bytecode[m.pc + 1..]);
            Ok(ControlFlow::Continue)
        }

        PUSH10(m) {
            m.pc += push::<10>(&mut m.runtime.stack, &m.bytecode[m.pc + 1..]);
            Ok(ControlFlow::Continue)
        }

        PUSH11(m) {
            m.pc += push::<11>(&mut m.runtime.stack, &m.bytecode[m.pc + 1..]);
            Ok(ControlFlow::Continue)
        }

        PUSH12(m) {
            m.pc += push::<12>(&mut m.runtime.stack, &m.bytecode[m.pc + 1..]);
            Ok(ControlFlow::Continue)
        }

        PUSH13(m) {
            m.pc += push::<13>(&mut m.runtime.stack, &m.bytecode[m.pc + 1..]);
            Ok(ControlFlow::Continue)
        }

        PUSH14(m) {
            m.pc += push::<14>(&mut m.runtime.stack, &m.bytecode[m.pc + 1..]);
            Ok(ControlFlow::Continue)
        }

        PUSH15(m) {
            m.pc += push::<15>(&mut m.runtime.stack, &m.bytecode[m.pc + 1..]);
            Ok(ControlFlow::Continue)
        }

        PUSH16(m) {
            m.pc += push::<16>(&mut m.runtime.stack, &m.bytecode[m.pc + 1..]);
            Ok(ControlFlow::Continue)
        }

        PUSH17(m) {
            m.pc += push::<17>(&mut m.runtime.stack, &m.bytecode[m.pc + 1..]);
            Ok(ControlFlow::Continue)
        }

        PUSH18(m) {
            m.pc += push::<18>(&mut m.runtime.stack, &m.bytecode[m.pc + 1..]);
            Ok(ControlFlow::Continue)
        }

        PUSH19(m) {
            m.pc += push::<19>(&mut m.runtime.stack, &m.bytecode[m.pc + 1..]);
            Ok(ControlFlow::Continue)
        }

        PUSH20(m) {
            m.pc += push::<20>(&mut m.runtime.stack, &m.bytecode[m.pc + 1..]);
            Ok(ControlFlow::Continue)
        }

        PUSH21(m) {
            m.pc += push::<21>(&mut m.runtime.stack, &m.bytecode[m.pc + 1..]);
            Ok(ControlFlow::Continue)
        }

        PUSH22(m) {
            m.pc += push::<22>(&mut m.runtime.stack, &m.bytecode[m.pc + 1..]);
            Ok(ControlFlow::Continue)
        }

        PUSH23(m) {
            m.pc += push::<23>(&mut m.runtime.stack, &m.bytecode[m.pc + 1..]);
            Ok(ControlFlow::Continue)
        }

        PUSH24(m) {
            m.pc += push::<24>(&mut m.runtime.stack, &m.bytecode[m.pc + 1..]);
            Ok(ControlFlow::Continue)
        }

        PUSH25(m) {
            m.pc += push::<25>(&mut m.runtime.stack, &m.bytecode[m.pc + 1..]);
            Ok(ControlFlow::Continue)
        }

        PUSH26(m) {
            m.pc += push::<26>(&mut m.runtime.stack, &m.bytecode[m.pc + 1..]);
            Ok(ControlFlow::Continue)
        }

        PUSH27(m) {
            m.pc += push::<27>(&mut m.runtime.stack, &m.bytecode[m.pc + 1..]);
            Ok(ControlFlow::Continue)
        }

        PUSH28(m) {
            m.pc += push::<28>(&mut m.runtime.stack, &m.bytecode[m.pc + 1..]);
            Ok(ControlFlow::Continue)
        }

        PUSH29(m) {
            m.pc += push::<29>(&mut m.runtime.stack, &m.bytecode[m.pc + 1..]);
            Ok(ControlFlow::Continue)
        }

        PUSH30(m) {
            m.pc += push::<30>(&mut m.runtime.stack, &m.bytecode[m.pc + 1..]);
            Ok(ControlFlow::Continue)
        }

        PUSH31(m) {
            m.pc += push::<31>(&mut m.runtime.stack, &m.bytecode[m.pc + 1..]);
            Ok(ControlFlow::Continue)
        }

        PUSH32(m) {
            m.pc += push::<32>(&mut m.runtime.stack, &m.bytecode[m.pc + 1..]);
            Ok(ControlFlow::Continue)
        }

        DUP1(m) {
            dup::<1>(&mut m.runtime.stack);
            Ok(ControlFlow::Continue)
        }

        DUP2(m) {
            dup::<2>(&mut m.runtime.stack);
            Ok(ControlFlow::Continue)
        }

        DUP3(m) {
            dup::<3>(&mut m.runtime.stack);
            Ok(ControlFlow::Continue)
        }

        DUP4(m) {
            dup::<4>(&mut m.runtime.stack);
            Ok(ControlFlow::Continue)
        }

        DUP5(m) {
            dup::<5>(&mut m.runtime.stack);
            Ok(ControlFlow::Continue)
        }

        DUP6(m) {
            dup::<6>(&mut m.runtime.stack);
            Ok(ControlFlow::Continue)
        }

        DUP7(m) {
            dup::<7>(&mut m.runtime.stack);
            Ok(ControlFlow::Continue)
        }

        DUP8(m) {
            dup::<8>(&mut m.runtime.stack);
            Ok(ControlFlow::Continue)
        }

        DUP9(m) {
            dup::<9>(&mut m.runtime.stack);
            Ok(ControlFlow::Continue)
        }

        DUP10(m) {
            dup::<10>(&mut m.runtime.stack);
            Ok(ControlFlow::Continue)
        }

        DUP11(m) {
            dup::<11>(&mut m.runtime.stack);
            Ok(ControlFlow::Continue)
        }

        DUP12(m) {
            dup::<12>(&mut m.runtime.stack);
            Ok(ControlFlow::Continue)
        }

        DUP13(m) {
            dup::<13>(&mut m.runtime.stack);
            Ok(ControlFlow::Continue)
        }

        DUP14(m) {
            dup::<14>(&mut m.runtime.stack);
            Ok(ControlFlow::Continue)
        }

        DUP15(m) {
            dup::<15>(&mut m.runtime.stack);
            Ok(ControlFlow::Continue)
        }

        DUP16(m) {
            dup::<16>(&mut m.runtime.stack);
            Ok(ControlFlow::Continue)
        }

        SWAP1(m) {
            swap::<1>(&mut m.runtime.stack);
            Ok(ControlFlow::Continue)
        }

        SWAP2(m) {
            swap::<2>(&mut m.runtime.stack);
            Ok(ControlFlow::Continue)
        }

        SWAP3(m) {
            swap::<3>(&mut m.runtime.stack);
            Ok(ControlFlow::Continue)
        }

        SWAP4(m) {
            swap::<4>(&mut m.runtime.stack);
            Ok(ControlFlow::Continue)
        }

        SWAP5(m) {
            swap::<5>(&mut m.runtime.stack);
            Ok(ControlFlow::Continue)
        }

        SWAP6(m) {
            swap::<6>(&mut m.runtime.stack);
            Ok(ControlFlow::Continue)
        }

        SWAP7(m) {
            swap::<7>(&mut m.runtime.stack);
            Ok(ControlFlow::Continue)
        }

        SWAP8(m) {
            swap::<8>(&mut m.runtime.stack);
            Ok(ControlFlow::Continue)
        }

        SWAP9(m) {
            swap::<9>(&mut m.runtime.stack);
            Ok(ControlFlow::Continue)
        }

        SWAP10(m) {
            swap::<10>(&mut m.runtime.stack);
            Ok(ControlFlow::Continue)
        }

        SWAP11(m) {
            swap::<11>(&mut m.runtime.stack);
            Ok(ControlFlow::Continue)
        }

        SWAP12(m) {
            swap::<12>(&mut m.runtime.stack);
            Ok(ControlFlow::Continue)
        }

        SWAP13(m) {
            swap::<13>(&mut m.runtime.stack);
            Ok(ControlFlow::Continue)
        }

        SWAP14(m) {
            swap::<14>(&mut m.runtime.stack);
            Ok(ControlFlow::Continue)
        }

        SWAP15(m) {
            swap::<15>(&mut m.runtime.stack);
            Ok(ControlFlow::Continue)
        }

        SWAP16(m) {
            swap::<16>(&mut m.runtime.stack);
            Ok(ControlFlow::Continue)
        }

        LOG0(m) {
            log(m.runtime, m.system, 0)?;
            Ok(ControlFlow::Continue)
        }

        LOG1(m) {
            log(m.runtime, m.system, 1)?;
            Ok(ControlFlow::Continue)
        }

        LOG2(m) {
            log(m.runtime, m.system, 2)?;
            Ok(ControlFlow::Continue)
        }

        LOG3(m) {
            log(m.runtime, m.system, 3)?;
            Ok(ControlFlow::Continue)
        }

        LOG4(m) {
            log(m.runtime, m.system, 4)?;
            Ok(ControlFlow::Continue)
        }

        CREATE(m) {
            lifecycle::create(m.runtime, m.system, false)?;
            Ok(ControlFlow::Continue)
        }

        CALL(m) {
            call::call(m.runtime, m.system, CallKind::Call, false)?;
            Ok(ControlFlow::Continue)
        }

        CALLCODE(m) {
            call::call(m.runtime, m.system, CallKind::CallCode, false)?;
            Ok(ControlFlow::Continue)
        }

        RETURN(m) {
            control::ret(m.runtime)?;
            Ok(ControlFlow::Exit)
        }

        DELEGATECALL(m) {
            call::call(m.runtime, m.system, CallKind::DelegateCall, false)?;
            Ok(ControlFlow::Continue)
        }

        CREATE2(m) {
            lifecycle::create(m.runtime, m.system, true)?;
            Ok(ControlFlow::Continue)
        }

        STATICCALL(m) {
            call::call(m.runtime, m.system, CallKind::Call, true)?;
            Ok(ControlFlow::Continue)
        }

        REVERT(m) {
            control::ret(m.runtime)?;
            m.reverted = true;
            Ok(ControlFlow::Exit)
        }

        INVALID(_m) {
            Err(StatusCode::InvalidInstruction)
        }

        SELFDESTRUCT(m) {
            lifecycle::selfdestruct(m.runtime, m.system)?;
            Ok(ControlFlow::Continue)
        }
    }

    const JMPTABLE: [Instruction<Machine<'r, BS, RT>>; 256] = Machine::<'r, BS, RT>::jmptable();
}

pub fn execute<'r, BS: Blockstore, RT: Runtime<BS>>(
    bytecode: &'r Bytecode,
    runtime: &'r mut ExecutionState,
    system: &'r System<'r, BS, RT>,
) -> Result<Output, StatusCode> {
    let mut m = Machine::new(system, runtime, bytecode);
    m.execute()?;
    Ok(Output {
        reverted: m.reverted,
        status_code: StatusCode::Success,
        output_data: m.runtime.output_data.clone(),
    })
}
