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
    ($($op:ident: $ins:ident),*) => {
        let mut table: [Instruction<Machine::<'r, BS, RT>>; 256] = [Machine::<'r, BS, RT>::ins_undefined; 256];
        $(table[OpCode::$op as usize] = Machine::<'r, BS, RT>::$ins;)*
        table
    }
}

macro_rules! def_ins1 {
    ($ins:ident ($arg:ident) $body:block) => {
        fn $ins(p: *mut Self) -> Result<ControlFlow, StatusCode> {
            let $arg: &mut Self = unsafe { p.as_mut().unwrap() };
            $body
        }
    }
}

macro_rules! def_ins {
    ($($ins:ident ($arg:ident) $body:block)*) => {
        $(def_ins1! { $ins($arg) $body })*
    }
}

impl<'r, BS: Blockstore + 'r, RT: Runtime<BS> + 'r> Machine<'r, BS, RT> {
    pub fn new(system: &'r System<'r, BS, RT>,
               runtime: &'r mut ExecutionState,
               bytecode: &'r Bytecode,
    ) -> Self {
        Machine {
            system: system,
            runtime: runtime,
            bytecode: bytecode,
            pc: 0,
            reverted: false,
        }
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

    const fn jmptable() -> [Instruction<Machine<'r, BS, RT>>; 256] {
        def_jmptable! {
            STOP: ins_stop,
            ADD: ins_add,
            MUL: ins_mul,
            SUB: ins_sub,
            DIV: ins_div,
            SDIV: ins_sdiv,
            MOD: ins_mod,
            SMOD: ins_smod,
            ADDMOD: ins_addmod,
            MULMOD: ins_mulmod,
            EXP: ins_exp,
            SIGNEXTEND: ins_signextend,
            LT: ins_lt,
            GT: ins_gt,
            SLT: ins_slt,
            SGT: ins_sgt,
            EQ: ins_eq,
            ISZERO: ins_iszero,
            AND: ins_and,
            OR: ins_or,
            XOR: ins_xor,
            NOT: ins_not,
            BYTE: ins_byte,
            SHL: ins_shl,
            SHR: ins_shr,
            SAR: ins_sar,
            KECCAK256: ins_keccak256,
            ADDRESS: ins_address,
            BALANCE: ins_balance,
            ORIGIN: ins_origin,
            CALLER: ins_caller,
            CALLVALUE: ins_callvalue,
            CALLDATALOAD: ins_calldataload,
            CALLDATASIZE: ins_calldatasize,
            CALLDATACOPY: ins_calldatacopy,
            CODESIZE: ins_codesize,
            CODECOPY: ins_codecopy,
            GASPRICE: ins_gasprice,
            EXTCODESIZE: ins_extcodesize,
            EXTCODECOPY: ins_extcodecopy,
            RETURNDATASIZE: ins_returndatasize,
            RETURNDATACOPY: ins_returndatacopy,
            EXTCODEHASH: ins_extcodehash,
            BLOCKHASH: ins_blockhash,
            COINBASE: ins_coinbase,
            TIMESTAMP: ins_timestamp,
            NUMBER: ins_number,
            DIFFICULTY: ins_difficulty,
            GASLIMIT: ins_gaslimit,
            CHAINID: ins_chainid,
            SELFBALANCE: ins_selfbalance,
            BASEFEE: ins_basefee,
            POP: ins_pop,
            MLOAD: ins_mload,
            MSTORE: ins_mstore,
            MSTORE8: ins_mstore8,
            SLOAD: ins_sload,
            SSTORE: ins_sstore,
            JUMP: ins_jump,
            JUMPI: ins_jumpi,
            PC: ins_pc,
            MSIZE: ins_msize,
            GAS: ins_gas,
            JUMPDEST: ins_jumpdest,
            PUSH1: ins_push1,
            PUSH2: ins_push2,
            PUSH3: ins_push3,
            PUSH4: ins_push4,
            PUSH5: ins_push5,
            PUSH6: ins_push6,
            PUSH7: ins_push7,
            PUSH8: ins_push8,
            PUSH9: ins_push9,
            PUSH10: ins_push10,
            PUSH11: ins_push11,
            PUSH12: ins_push12,
            PUSH13: ins_push13,
            PUSH14: ins_push14,
            PUSH15: ins_push15,
            PUSH16: ins_push16,
            PUSH17: ins_push17,
            PUSH18: ins_push18,
            PUSH19: ins_push19,
            PUSH20: ins_push20,
            PUSH21: ins_push21,
            PUSH22: ins_push22,
            PUSH23: ins_push23,
            PUSH24: ins_push24,
            PUSH25: ins_push25,
            PUSH26: ins_push26,
            PUSH27: ins_push27,
            PUSH28: ins_push28,
            PUSH29: ins_push29,
            PUSH30: ins_push30,
            PUSH31: ins_push31,
            PUSH32: ins_push32,
            DUP1: ins_dup1,
            DUP2: ins_dup2,
            DUP3: ins_dup3,
            DUP4: ins_dup4,
            DUP5: ins_dup5,
            DUP6: ins_dup6,
            DUP7: ins_dup7,
            DUP8: ins_dup8,
            DUP9: ins_dup9,
            DUP10: ins_dup10,
            DUP11: ins_dup11,
            DUP12: ins_dup12,
            DUP13: ins_dup13,
            DUP14: ins_dup14,
            DUP15: ins_dup15,
            DUP16: ins_dup16,
            SWAP1: ins_swap1,
            SWAP2: ins_swap2,
            SWAP3: ins_swap3,
            SWAP4: ins_swap4,
            SWAP5: ins_swap5,
            SWAP6: ins_swap6,
            SWAP7: ins_swap7,
            SWAP8: ins_swap8,
            SWAP9: ins_swap9,
            SWAP10: ins_swap10,
            SWAP11: ins_swap11,
            SWAP12: ins_swap12,
            SWAP13: ins_swap13,
            SWAP14: ins_swap14,
            SWAP15: ins_swap15,
            SWAP16: ins_swap16,
            LOG0: ins_log0,
            LOG1: ins_log1,
            LOG2: ins_log2,
            LOG3: ins_log3,
            LOG4: ins_log4,
            CREATE: ins_create,
            CALL: ins_call,
            CALLCODE: ins_callcode,
            RETURN: ins_return,
            DELEGATECALL: ins_delegatecall,
            CREATE2: ins_create2,
            STATICCALL: ins_staticcall,
            REVERT: ins_revert,
            INVALID: ins_invalid,
            SELFDESTRUCT: ins_selfdestruct
        }
    }

    const JMPTABLE: [Instruction<Machine<'r, BS, RT>>; 256] = Machine::<'r, BS, RT>::jmptable();

    def_ins! {
        ins_undefined(_m) {
            Err(StatusCode::UndefinedInstruction)
        }

        ins_stop(_m) {
            Ok(ControlFlow::Exit)
        }

        ins_add(m) {
            arithmetic::add(&mut m.runtime.stack);
            Ok(ControlFlow::Continue)
        }

        ins_mul(m) {
            arithmetic::mul(&mut m.runtime.stack);
            Ok(ControlFlow::Continue)
        }

        ins_sub(m) {
            arithmetic::sub(&mut m.runtime.stack);
            Ok(ControlFlow::Continue)
        }

        ins_div(m) {
            arithmetic::div(&mut m.runtime.stack);
            Ok(ControlFlow::Continue)
        }

        ins_sdiv(m) {
            arithmetic::sdiv(&mut m.runtime.stack);
            Ok(ControlFlow::Continue)
        }

        ins_mod(m) {
            arithmetic::modulo(&mut m.runtime.stack);
            Ok(ControlFlow::Continue)
        }

        ins_smod(m) {
            arithmetic::smod(&mut m.runtime.stack);
            Ok(ControlFlow::Continue)
        }

        ins_addmod(m) {
            arithmetic::addmod(&mut m.runtime.stack);
            Ok(ControlFlow::Continue)
        }

        ins_mulmod(m) {
            arithmetic::mulmod(&mut m.runtime.stack);
            Ok(ControlFlow::Continue)
        }

        ins_exp(m) {
            arithmetic::exp(&mut m.runtime.stack);
            Ok(ControlFlow::Continue)
        }

        ins_signextend(m) {
            arithmetic::signextend(&mut m.runtime.stack);
            Ok(ControlFlow::Continue)
        }

        ins_lt(m) {
            boolean::lt(&mut m.runtime.stack);
            Ok(ControlFlow::Continue)
        }

        ins_gt(m) {
            boolean::gt(&mut m.runtime.stack);
            Ok(ControlFlow::Continue)
        }

        ins_slt(m) {
            boolean::slt(&mut m.runtime.stack);
            Ok(ControlFlow::Continue)
        }

        ins_sgt(m) {
            boolean::sgt(&mut m.runtime.stack);
            Ok(ControlFlow::Continue)
        }

        ins_eq(m) {
            boolean::eq(&mut m.runtime.stack);
            Ok(ControlFlow::Continue)
        }

        ins_iszero(m) {
            boolean::iszero(&mut m.runtime.stack);
            Ok(ControlFlow::Continue)
        }

        ins_and(m) {
            boolean::and(&mut m.runtime.stack);
            Ok(ControlFlow::Continue)
        }

        ins_or(m) {
            boolean::or(&mut m.runtime.stack);
            Ok(ControlFlow::Continue)
        }

        ins_xor(m) {
            boolean::xor(&mut m.runtime.stack);
            Ok(ControlFlow::Continue)
        }

        ins_not(m) {
            boolean::not(&mut m.runtime.stack);
            Ok(ControlFlow::Continue)
        }

        ins_byte(m) {
            bitwise::byte(&mut m.runtime.stack);
            Ok(ControlFlow::Continue)
        }

        ins_shl(m) {
            bitwise::shl(&mut m.runtime.stack);
            Ok(ControlFlow::Continue)
        }

        ins_shr(m) {
            bitwise::shr(&mut m.runtime.stack);
            Ok(ControlFlow::Continue)
        }

        ins_sar(m) {
            bitwise::sar(&mut m.runtime.stack);
            Ok(ControlFlow::Continue)
        }

        ins_keccak256(m) {
            hash::keccak256(m.runtime)?;
            Ok(ControlFlow::Continue)
        }

        ins_address(m) {
            context::address(m.runtime, m.system);
            Ok(ControlFlow::Continue)
        }

        ins_balance(m) {
            storage::balance(m.runtime, m.system)?;
            Ok(ControlFlow::Continue)
        }

        ins_origin(m) {
            context::origin(m.runtime, m.system);
            Ok(ControlFlow::Continue)
        }

        ins_caller(m) {
            context::caller(m.runtime, m.system);
            Ok(ControlFlow::Continue)
        }

        ins_callvalue(m) {
            context::call_value(m.runtime, m.system);
            Ok(ControlFlow::Continue)
        }

        ins_calldataload(m) {
            call::calldataload(m.runtime);
            Ok(ControlFlow::Continue)
        }

        ins_calldatasize(m) {
            call::calldatasize(m.runtime);
            Ok(ControlFlow::Continue)
        }

        ins_calldatacopy(m) {
            call::calldatacopy(m.runtime)?;
            Ok(ControlFlow::Continue)
        }

        ins_codesize(m) {
            call::codesize(&mut m.runtime.stack, m.bytecode.as_ref());
            Ok(ControlFlow::Continue)
        }

        ins_codecopy(m) {
            call::codecopy(m.runtime, m.bytecode.as_ref())?;
            Ok(ControlFlow::Continue)
        }

        ins_gasprice(m) {
            context::gas_price(m.runtime, m.system)?;
            Ok(ControlFlow::Continue)
        }

        ins_extcodesize(m) {
            storage::extcodesize(m.runtime, m.system)?;
            Ok(ControlFlow::Continue)
        }

        ins_extcodecopy(m) {
            memory::extcodecopy(m.runtime, m.system)?;
            Ok(ControlFlow::Continue)
        }

        ins_returndatasize(m) {
            control::returndatasize(m.runtime);
            Ok(ControlFlow::Continue)
        }

        ins_returndatacopy(m) {
            control::returndatacopy(m.runtime)?;
            Ok(ControlFlow::Continue)
        }

        ins_extcodehash(m) {
            storage::extcodehash(m.runtime, m.system)?;
            Ok(ControlFlow::Continue)
        }

        ins_blockhash(m) {
            context::blockhash(m.runtime, m.system)?;
            Ok(ControlFlow::Continue)
        }

        ins_coinbase(m) {
            context::coinbase(m.runtime, m.system)?;
            Ok(ControlFlow::Continue)
        }

        ins_timestamp(m) {
            context::timestamp(m.runtime, m.system)?;
            Ok(ControlFlow::Continue)
        }

        ins_number(m) {
            context::block_number(m.runtime, m.system)?;
            Ok(ControlFlow::Continue)
        }

        ins_difficulty(m) {
            context::difficulty(m.runtime, m.system)?;
            Ok(ControlFlow::Continue)
        }

        ins_gaslimit(m) {
            context::gas_limit(m.runtime, m.system)?;
            Ok(ControlFlow::Continue)
        }

        ins_chainid(m) {
            context::chain_id(m.runtime, m.system)?;
            Ok(ControlFlow::Continue)
        }

        ins_selfbalance(m) {
            storage::selfbalance(m.runtime, m.system)?;
            Ok(ControlFlow::Continue)
        }

        ins_basefee(m) {
            context::base_fee(m.runtime, m.system)?;
            Ok(ControlFlow::Continue)
        }

        ins_pop(m) {
            stack::pop(&mut m.runtime.stack);
            Ok(ControlFlow::Continue)
        }

        ins_mload(m) {
            memory::mload(m.runtime)?;
            Ok(ControlFlow::Continue)
        }

        ins_mstore(m) {
            memory::mstore(m.runtime)?;
            Ok(ControlFlow::Continue)
        }

        ins_mstore8(m) {
            memory::mstore8(m.runtime)?;
            Ok(ControlFlow::Continue)
        }

        ins_sload(m) {
            storage::sload(m.runtime, m.system)?;
            Ok(ControlFlow::Continue)
        }

        ins_sstore(m) {
            storage::sstore(m.runtime, m.system)?;
            Ok(ControlFlow::Continue)
        }

        ins_jump(m) {
            m.pc = control::jump(&mut m.runtime.stack, m.bytecode)?;
            Ok(ControlFlow::Jump)
        }

        ins_jumpi(m) {
            if let Some(dest) = control::jumpi(&mut m.runtime.stack, m.bytecode)? {
                m.pc = dest;
                Ok(ControlFlow::Jump)
            } else {
                Ok(ControlFlow::Continue)
            }
        }

        ins_pc(m) {
            control::pc(&mut m.runtime.stack, m.pc);
            Ok(ControlFlow::Continue)
        }

        ins_msize(m) {
            memory::msize(m.runtime);
            Ok(ControlFlow::Continue)
        }

        ins_gas(m) {
            control::gas(m.runtime);
            Ok(ControlFlow::Continue)
        }

        ins_jumpdest(_m) {
            // marker opcode for valid jumps addresses
            Ok(ControlFlow::Continue)
        }

        ins_push1(m) {
            m.pc += push::<1>(&mut m.runtime.stack, &m.bytecode[m.pc + 1..]);
            Ok(ControlFlow::Continue)
        }

        ins_push2(m) {
            m.pc += push::<2>(&mut m.runtime.stack, &m.bytecode[m.pc + 1..]);
            Ok(ControlFlow::Continue)
        }

        ins_push3(m) {
            m.pc += push::<3>(&mut m.runtime.stack, &m.bytecode[m.pc + 1..]);
            Ok(ControlFlow::Continue)
        }

        ins_push4(m) {
            m.pc += push::<4>(&mut m.runtime.stack, &m.bytecode[m.pc + 1..]);
            Ok(ControlFlow::Continue)
        }

        ins_push5(m) {
            m.pc += push::<5>(&mut m.runtime.stack, &m.bytecode[m.pc + 1..]);
            Ok(ControlFlow::Continue)
        }

        ins_push6(m) {
            m.pc += push::<6>(&mut m.runtime.stack, &m.bytecode[m.pc + 1..]);
            Ok(ControlFlow::Continue)
        }

        ins_push7(m) {
            m.pc += push::<7>(&mut m.runtime.stack, &m.bytecode[m.pc + 1..]);
            Ok(ControlFlow::Continue)
        }

        ins_push8(m) {
            m.pc += push::<8>(&mut m.runtime.stack, &m.bytecode[m.pc + 1..]);
            Ok(ControlFlow::Continue)
        }

        ins_push9(m) {
            m.pc += push::<9>(&mut m.runtime.stack, &m.bytecode[m.pc + 1..]);
            Ok(ControlFlow::Continue)
        }

        ins_push10(m) {
            m.pc += push::<10>(&mut m.runtime.stack, &m.bytecode[m.pc + 1..]);
            Ok(ControlFlow::Continue)
        }

        ins_push11(m) {
            m.pc += push::<11>(&mut m.runtime.stack, &m.bytecode[m.pc + 1..]);
            Ok(ControlFlow::Continue)
        }

        ins_push12(m) {
            m.pc += push::<12>(&mut m.runtime.stack, &m.bytecode[m.pc + 1..]);
            Ok(ControlFlow::Continue)
        }

        ins_push13(m) {
            m.pc += push::<13>(&mut m.runtime.stack, &m.bytecode[m.pc + 1..]);
            Ok(ControlFlow::Continue)
        }

        ins_push14(m) {
            m.pc += push::<14>(&mut m.runtime.stack, &m.bytecode[m.pc + 1..]);
            Ok(ControlFlow::Continue)
        }

        ins_push15(m) {
            m.pc += push::<15>(&mut m.runtime.stack, &m.bytecode[m.pc + 1..]);
            Ok(ControlFlow::Continue)
        }

        ins_push16(m) {
            m.pc += push::<16>(&mut m.runtime.stack, &m.bytecode[m.pc + 1..]);
            Ok(ControlFlow::Continue)
        }

        ins_push17(m) {
            m.pc += push::<17>(&mut m.runtime.stack, &m.bytecode[m.pc + 1..]);
            Ok(ControlFlow::Continue)
        }

        ins_push18(m) {
            m.pc += push::<18>(&mut m.runtime.stack, &m.bytecode[m.pc + 1..]);
            Ok(ControlFlow::Continue)
        }

        ins_push19(m) {
            m.pc += push::<19>(&mut m.runtime.stack, &m.bytecode[m.pc + 1..]);
            Ok(ControlFlow::Continue)
        }

        ins_push20(m) {
            m.pc += push::<20>(&mut m.runtime.stack, &m.bytecode[m.pc + 1..]);
            Ok(ControlFlow::Continue)
        }

        ins_push21(m) {
            m.pc += push::<21>(&mut m.runtime.stack, &m.bytecode[m.pc + 1..]);
            Ok(ControlFlow::Continue)
        }

        ins_push22(m) {
            m.pc += push::<22>(&mut m.runtime.stack, &m.bytecode[m.pc + 1..]);
            Ok(ControlFlow::Continue)
        }

        ins_push23(m) {
            m.pc += push::<23>(&mut m.runtime.stack, &m.bytecode[m.pc + 1..]);
            Ok(ControlFlow::Continue)
        }

        ins_push24(m) {
            m.pc += push::<24>(&mut m.runtime.stack, &m.bytecode[m.pc + 1..]);
            Ok(ControlFlow::Continue)
        }

        ins_push25(m) {
            m.pc += push::<25>(&mut m.runtime.stack, &m.bytecode[m.pc + 1..]);
            Ok(ControlFlow::Continue)
        }

        ins_push26(m) {
            m.pc += push::<26>(&mut m.runtime.stack, &m.bytecode[m.pc + 1..]);
            Ok(ControlFlow::Continue)
        }

        ins_push27(m) {
            m.pc += push::<27>(&mut m.runtime.stack, &m.bytecode[m.pc + 1..]);
            Ok(ControlFlow::Continue)
        }

        ins_push28(m) {
            m.pc += push::<28>(&mut m.runtime.stack, &m.bytecode[m.pc + 1..]);
            Ok(ControlFlow::Continue)
        }

        ins_push29(m) {
            m.pc += push::<29>(&mut m.runtime.stack, &m.bytecode[m.pc + 1..]);
            Ok(ControlFlow::Continue)
        }

        ins_push30(m) {
            m.pc += push::<30>(&mut m.runtime.stack, &m.bytecode[m.pc + 1..]);
            Ok(ControlFlow::Continue)
        }

        ins_push31(m) {
            m.pc += push::<31>(&mut m.runtime.stack, &m.bytecode[m.pc + 1..]);
            Ok(ControlFlow::Continue)
        }

        ins_push32(m) {
            m.pc += push::<32>(&mut m.runtime.stack, &m.bytecode[m.pc + 1..]);
            Ok(ControlFlow::Continue)
        }

        ins_dup1(m) {
            dup::<1>(&mut m.runtime.stack);
            Ok(ControlFlow::Continue)
        }

        ins_dup2(m) {
            dup::<2>(&mut m.runtime.stack);
            Ok(ControlFlow::Continue)
        }

        ins_dup3(m) {
            dup::<3>(&mut m.runtime.stack);
            Ok(ControlFlow::Continue)
        }

        ins_dup4(m) {
            dup::<4>(&mut m.runtime.stack);
            Ok(ControlFlow::Continue)
        }

        ins_dup5(m) {
            dup::<5>(&mut m.runtime.stack);
            Ok(ControlFlow::Continue)
        }

        ins_dup6(m) {
            dup::<6>(&mut m.runtime.stack);
            Ok(ControlFlow::Continue)
        }

        ins_dup7(m) {
            dup::<7>(&mut m.runtime.stack);
            Ok(ControlFlow::Continue)
        }

        ins_dup8(m) {
            dup::<8>(&mut m.runtime.stack);
            Ok(ControlFlow::Continue)
        }

        ins_dup9(m) {
            dup::<9>(&mut m.runtime.stack);
            Ok(ControlFlow::Continue)
        }

        ins_dup10(m) {
            dup::<10>(&mut m.runtime.stack);
            Ok(ControlFlow::Continue)
        }

        ins_dup11(m) {
            dup::<11>(&mut m.runtime.stack);
            Ok(ControlFlow::Continue)
        }

        ins_dup12(m) {
            dup::<12>(&mut m.runtime.stack);
            Ok(ControlFlow::Continue)
        }

        ins_dup13(m) {
            dup::<13>(&mut m.runtime.stack);
            Ok(ControlFlow::Continue)
        }

        ins_dup14(m) {
            dup::<14>(&mut m.runtime.stack);
            Ok(ControlFlow::Continue)
        }

        ins_dup15(m) {
            dup::<15>(&mut m.runtime.stack);
            Ok(ControlFlow::Continue)
        }

        ins_dup16(m) {
            dup::<16>(&mut m.runtime.stack);
            Ok(ControlFlow::Continue)
        }

        ins_swap1(m) {
            swap::<1>(&mut m.runtime.stack);
            Ok(ControlFlow::Continue)
        }

        ins_swap2(m) {
            swap::<2>(&mut m.runtime.stack);
            Ok(ControlFlow::Continue)
        }

        ins_swap3(m) {
            swap::<3>(&mut m.runtime.stack);
            Ok(ControlFlow::Continue)
        }

        ins_swap4(m) {
            swap::<4>(&mut m.runtime.stack);
            Ok(ControlFlow::Continue)
        }

        ins_swap5(m) {
            swap::<5>(&mut m.runtime.stack);
            Ok(ControlFlow::Continue)
        }

        ins_swap6(m) {
            swap::<6>(&mut m.runtime.stack);
            Ok(ControlFlow::Continue)
        }

        ins_swap7(m) {
            swap::<7>(&mut m.runtime.stack);
            Ok(ControlFlow::Continue)
        }

        ins_swap8(m) {
            swap::<8>(&mut m.runtime.stack);
            Ok(ControlFlow::Continue)
        }

        ins_swap9(m) {
            swap::<9>(&mut m.runtime.stack);
            Ok(ControlFlow::Continue)
        }

        ins_swap10(m) {
            swap::<10>(&mut m.runtime.stack);
            Ok(ControlFlow::Continue)
        }

        ins_swap11(m) {
            swap::<11>(&mut m.runtime.stack);
            Ok(ControlFlow::Continue)
        }

        ins_swap12(m) {
            swap::<12>(&mut m.runtime.stack);
            Ok(ControlFlow::Continue)
        }

        ins_swap13(m) {
            swap::<13>(&mut m.runtime.stack);
            Ok(ControlFlow::Continue)
        }

        ins_swap14(m) {
            swap::<14>(&mut m.runtime.stack);
            Ok(ControlFlow::Continue)
        }

        ins_swap15(m) {
            swap::<15>(&mut m.runtime.stack);
            Ok(ControlFlow::Continue)
        }

        ins_swap16(m) {
            swap::<16>(&mut m.runtime.stack);
            Ok(ControlFlow::Continue)
        }

        ins_log0(m) {
            log(m.runtime, m.system, 0)?;
            Ok(ControlFlow::Continue)
        }

        ins_log1(m) {
            log(m.runtime, m.system, 1)?;
            Ok(ControlFlow::Continue)
        }

        ins_log2(m) {
            log(m.runtime, m.system, 2)?;
            Ok(ControlFlow::Continue)
        }

        ins_log3(m) {
            log(m.runtime, m.system, 3)?;
            Ok(ControlFlow::Continue)
        }

        ins_log4(m) {
            log(m.runtime, m.system, 4)?;
            Ok(ControlFlow::Continue)
        }

        ins_create(m) {
            storage::create(m.runtime, m.system, false)?;
            Ok(ControlFlow::Continue)
        }

        ins_call(m) {
            call::call(m.runtime, m.system, CallKind::Call, false)?;
            Ok(ControlFlow::Continue)
        }

        ins_callcode(m) {
            call::call(m.runtime, m.system, CallKind::CallCode, false)?;
            Ok(ControlFlow::Continue)
        }

        ins_return(m) {
            control::ret(m.runtime)?;
            Ok(ControlFlow::Exit)
        }

        ins_delegatecall(m) {
            call::call(m.runtime, m.system, CallKind::DelegateCall, false)?;
            Ok(ControlFlow::Continue)
        }

        ins_create2(m) {
            storage::create(m.runtime, m.system, true)?;
            Ok(ControlFlow::Continue)
        }

        ins_staticcall(m) {
            call::call(m.runtime, m.system, CallKind::Call, true)?;
            Ok(ControlFlow::Continue)
        }

        ins_revert(m) {
            control::ret(m.runtime)?;
            m.reverted = true;
            Ok(ControlFlow::Exit)
        }

        ins_invalid(_m) {
            Err(StatusCode::InvalidInstruction)
        }

        ins_selfdestruct(m) {
            storage::selfdestruct(m.runtime, m.system)?;
            Ok(ControlFlow::Continue)
        }
    }
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
