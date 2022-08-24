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
    Advance,
    Continue,
    Break,
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
            let $arg :&mut Self = unsafe { p.as_mut().unwrap() };
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
    fn new(system: &'r System<'r, BS, RT>,
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
            todo!()
        }

        ins_stop(_m) {
            todo!()
        }

        ins_add(_m) {
            todo!()
        }

        ins_mul(_m) {
            todo!()
        }

        ins_sub(_m) {
            todo!()
        }

        ins_div(_m) {
            todo!()
        }

        ins_sdiv(_m) {
            todo!()
        }

        ins_mod(_m) {
            todo!()
        }

        ins_smod(_m) {
            todo!()
        }

        ins_addmod(_m) {
            todo!()
        }

        ins_mulmod(_m) {
            todo!()
        }

        ins_exp(_m) {
            todo!()
        }

        ins_signextend(_m) {
            todo!()
        }

        ins_lt(_m) {
            todo!()
        }

        ins_gt(_m) {
            todo!()
        }

        ins_slt(_m) {
            todo!()
        }

        ins_sgt(_m) {
            todo!()
        }

        ins_eq(_m) {
            todo!()
        }

        ins_iszero(_m) {
            todo!()
        }

        ins_and(_m) {
            todo!()
        }

        ins_or(_m) {
            todo!()
        }

        ins_xor(_m) {
            todo!()
        }

        ins_not(_m) {
            todo!()
        }

        ins_byte(_m) {
            todo!()
        }

        ins_shl(_m) {
            todo!()
        }

        ins_shr(_m) {
            todo!()
        }

        ins_sar(_m) {
            todo!()
        }

        ins_keccak256(_m) {
            todo!()
        }

        ins_address(_m) {
            todo!()
        }

        ins_balance(_m) {
            todo!()
        }

        ins_origin(_m) {
            todo!()
        }

        ins_caller(_m) {
            todo!()
        }

        ins_callvalue(_m) {
            todo!()
        }

        ins_calldataload(_m) {
            todo!()
        }

        ins_calldatasize(_m) {
            todo!()
        }

        ins_calldatacopy(_m) {
            todo!()
        }

        ins_codesize(_m) {
            todo!()
        }

        ins_codecopy(_m) {
            todo!()
        }

        ins_gasprice(_m) {
            todo!()
        }

        ins_extcodesize(_m) {
            todo!()
        }

        ins_extcodecopy(_m) {
            todo!()
        }

        ins_returndatasize(_m) {
            todo!()
        }

        ins_returndatacopy(_m) {
            todo!()
        }

        ins_extcodehash(_m) {
            todo!()
        }

        ins_blockhash(_m) {
            todo!()
        }

        ins_coinbase(_m) {
            todo!()
        }

        ins_timestamp(_m) {
            todo!()
        }

        ins_number(_m) {
            todo!()
        }

        ins_difficulty(_m) {
            todo!()
        }

        ins_gaslimit(_m) {
            todo!()
        }

        ins_chainid(_m) {
            todo!()
        }

        ins_selfbalance(_m) {
            todo!()
        }

        ins_basefee(_m) {
            todo!()
        }

        ins_pop(_m) {
            todo!()
        }

        ins_mload(_m) {
            todo!()
        }

        ins_mstore(_m) {
            todo!()
        }

        ins_mstore8(_m) {
            todo!()
        }

        ins_sload(_m) {
            todo!()
        }

        ins_sstore(_m) {
            todo!()
        }

        ins_jump(_m) {
            todo!()
        }

        ins_jumpi(_m) {
            todo!()
        }

        ins_pc(_m) {
            todo!()
        }

        ins_msize(_m) {
            todo!()
        }

        ins_gas(_m) {
            todo!()
        }

        ins_jumpdest(_m) {
            todo!()
        }

        ins_push1(_m) {
            todo!()
        }

        ins_push2(_m) {
            todo!()
        }

        ins_push3(_m) {
            todo!()
        }

        ins_push4(_m) {
            todo!()
        }

        ins_push5(_m) {
            todo!()
        }

        ins_push6(_m) {
            todo!()
        }

        ins_push7(_m) {
            todo!()
        }

        ins_push8(_m) {
            todo!()
        }

        ins_push9(_m) {
            todo!()
        }

        ins_push10(_m) {
            todo!()
        }

        ins_push11(_m) {
            todo!()
        }

        ins_push12(_m) {
            todo!()
        }

        ins_push13(_m) {
            todo!()
        }

        ins_push14(_m) {
            todo!()
        }

        ins_push15(_m) {
            todo!()
        }

        ins_push16(_m) {
            todo!()
        }

        ins_push17(_m) {
            todo!()
        }

        ins_push18(_m) {
            todo!()
        }

        ins_push19(_m) {
            todo!()
        }

        ins_push20(_m) {
            todo!()
        }

        ins_push21(_m) {
            todo!()
        }

        ins_push22(_m) {
            todo!()
        }

        ins_push23(_m) {
            todo!()
        }

        ins_push24(_m) {
            todo!()
        }

        ins_push25(_m) {
            todo!()
        }

        ins_push26(_m) {
            todo!()
        }

        ins_push27(_m) {
            todo!()
        }

        ins_push28(_m) {
            todo!()
        }

        ins_push29(_m) {
            todo!()
        }

        ins_push30(_m) {
            todo!()
        }

        ins_push31(_m) {
            todo!()
        }

        ins_push32(_m) {
            todo!()
        }

        ins_dup1(_m) {
            todo!()
        }

        ins_dup2(_m) {
            todo!()
        }

        ins_dup3(_m) {
            todo!()
        }

        ins_dup4(_m) {
            todo!()
        }

        ins_dup5(_m) {
            todo!()
        }

        ins_dup6(_m) {
            todo!()
        }

        ins_dup7(_m) {
            todo!()
        }

        ins_dup8(_m) {
            todo!()
        }

        ins_dup9(_m) {
            todo!()
        }

        ins_dup10(_m) {
            todo!()
        }

        ins_dup11(_m) {
            todo!()
        }

        ins_dup12(_m) {
            todo!()
        }

        ins_dup13(_m) {
            todo!()
        }

        ins_dup14(_m) {
            todo!()
        }

        ins_dup15(_m) {
            todo!()
        }

        ins_dup16(_m) {
            todo!()
        }

        ins_swap1(_m) {
            todo!()
        }

        ins_swap2(_m) {
            todo!()
        }

        ins_swap3(_m) {
            todo!()
        }

        ins_swap4(_m) {
            todo!()
        }

        ins_swap5(_m) {
            todo!()
        }

        ins_swap6(_m) {
            todo!()
        }

        ins_swap7(_m) {
            todo!()
        }

        ins_swap8(_m) {
            todo!()
        }

        ins_swap9(_m) {
            todo!()
        }

        ins_swap10(_m) {
            todo!()
        }

        ins_swap11(_m) {
            todo!()
        }

        ins_swap12(_m) {
            todo!()
        }

        ins_swap13(_m) {
            todo!()
        }

        ins_swap14(_m) {
            todo!()
        }

        ins_swap15(_m) {
            todo!()
        }

        ins_swap16(_m) {
            todo!()
        }

        ins_log0(_m) {
            todo!()
        }

        ins_log1(_m) {
            todo!()
        }

        ins_log2(_m) {
            todo!()
        }

        ins_log3(_m) {
            todo!()
        }

        ins_log4(_m) {
            todo!()
        }

        ins_create(_m) {
            todo!()
        }

        ins_call(_m) {
            todo!()
        }

        ins_callcode(_m) {
            todo!()
        }

        ins_return(_m) {
            todo!()
        }

        ins_delegatecall(_m) {
            todo!()
        }

        ins_create2(_m) {
            todo!()
        }

        ins_staticcall(_m) {
            todo!()
        }

        ins_revert(_m) {
            todo!()
        }

        ins_invalid(_m) {
            todo!()
        }

        ins_selfdestruct(_m) {
            todo!()
        }
    }
}

pub fn execute<'r, BS: Blockstore, RT: Runtime<BS>>(
    bytecode: &Bytecode,
    runtime: &mut ExecutionState,
    system: &'r System<'r, BS, RT>,
) -> Result<Output, StatusCode> {
    let mut pc = 0; // program counter
    let mut reverted = false;

    loop {
        if pc >= bytecode.len() {
            break;
        }

        let op = OpCode::try_from(bytecode[pc])?;
        match op {
            OpCode::STOP => break,
            OpCode::ADD => arithmetic::add(&mut runtime.stack),
            OpCode::MUL => arithmetic::mul(&mut runtime.stack),
            OpCode::SUB => arithmetic::sub(&mut runtime.stack),
            OpCode::DIV => arithmetic::div(&mut runtime.stack),
            OpCode::SDIV => arithmetic::sdiv(&mut runtime.stack),
            OpCode::MOD => arithmetic::modulo(&mut runtime.stack),
            OpCode::SMOD => arithmetic::smod(&mut runtime.stack),
            OpCode::ADDMOD => arithmetic::addmod(&mut runtime.stack),
            OpCode::MULMOD => arithmetic::mulmod(&mut runtime.stack),
            OpCode::EXP => arithmetic::exp(runtime)?,
            OpCode::SIGNEXTEND => arithmetic::signextend(&mut runtime.stack),
            OpCode::LT => boolean::lt(&mut runtime.stack),
            OpCode::GT => boolean::gt(&mut runtime.stack),
            OpCode::SLT => boolean::slt(&mut runtime.stack),
            OpCode::SGT => boolean::sgt(&mut runtime.stack),
            OpCode::EQ => boolean::eq(&mut runtime.stack),
            OpCode::ISZERO => boolean::iszero(&mut runtime.stack),
            OpCode::AND => boolean::and(&mut runtime.stack),
            OpCode::OR => boolean::or(&mut runtime.stack),
            OpCode::XOR => boolean::xor(&mut runtime.stack),
            OpCode::NOT => boolean::not(&mut runtime.stack),
            OpCode::BYTE => bitwise::byte(&mut runtime.stack),
            OpCode::SHL => bitwise::shl(&mut runtime.stack),
            OpCode::SHR => bitwise::shr(&mut runtime.stack),
            OpCode::SAR => bitwise::sar(&mut runtime.stack),
            OpCode::KECCAK256 => hash::keccak256(runtime)?,
            OpCode::ADDRESS => context::address(runtime, system),
            OpCode::BALANCE => storage::balance(runtime, system)?,
            OpCode::CALLER => context::caller(runtime, system),
            OpCode::CALLVALUE => context::call_value(runtime, system),
            OpCode::CALLDATALOAD => call::calldataload(runtime),
            OpCode::CALLDATASIZE => call::calldatasize(runtime),
            OpCode::CALLDATACOPY => call::calldatacopy(runtime)?,
            OpCode::CODESIZE => call::codesize(&mut runtime.stack, bytecode.as_ref()),
            OpCode::CODECOPY => call::codecopy(runtime, bytecode.as_ref())?,
            OpCode::EXTCODESIZE => storage::extcodesize(runtime, system)?,
            OpCode::EXTCODECOPY => memory::extcodecopy(runtime, system)?,
            OpCode::RETURNDATASIZE => control::returndatasize(runtime),
            OpCode::RETURNDATACOPY => control::returndatacopy(runtime)?,
            OpCode::EXTCODEHASH => storage::extcodehash(runtime, system)?,
            OpCode::BLOCKHASH => context::blockhash(runtime, system)?,
            OpCode::ORIGIN => context::origin(runtime, system),
            OpCode::COINBASE => context::coinbase(runtime, system)?,
            OpCode::GASPRICE => context::gas_price(runtime, system)?,
            OpCode::TIMESTAMP => context::timestamp(runtime, system)?,
            OpCode::NUMBER => context::block_number(runtime, system)?,
            OpCode::DIFFICULTY => context::difficulty(runtime, system)?,
            OpCode::GASLIMIT => context::gas_limit(runtime, system)?,
            OpCode::CHAINID => context::chain_id(runtime, system)?,
            OpCode::BASEFEE => context::base_fee(runtime, system)?,
            OpCode::SELFBALANCE => storage::selfbalance(runtime, system)?,
            OpCode::POP => stack::pop(&mut runtime.stack),
            OpCode::MLOAD => memory::mload(runtime)?,
            OpCode::MSTORE => memory::mstore(runtime)?,
            OpCode::MSTORE8 => memory::mstore8(runtime)?,
            OpCode::JUMP => {
                pc = control::jump(&mut runtime.stack, bytecode)?;
                continue; // don't increment PC after the jump
            }
            OpCode::JUMPI => {
                // conditional jump
                if let Some(dest) = control::jumpi(&mut runtime.stack, bytecode)? {
                    pc = dest; // condition met, set program counter
                    continue; // don't increment PC after jump
                }
            }
            OpCode::PC => control::pc(&mut runtime.stack, pc),
            OpCode::MSIZE => memory::msize(runtime),
            OpCode::SLOAD => storage::sload(runtime, system)?,
            OpCode::SSTORE => storage::sstore(runtime, system)?,
            OpCode::GAS => control::gas(runtime),
            OpCode::JUMPDEST => {} // marker opcode for valid jumps addresses
            OpCode::PUSH1 => pc += push::<1>(&mut runtime.stack, &bytecode[pc + 1..]),
            OpCode::PUSH2 => pc += push::<2>(&mut runtime.stack, &bytecode[pc + 1..]),
            OpCode::PUSH3 => pc += push::<3>(&mut runtime.stack, &bytecode[pc + 1..]),
            OpCode::PUSH4 => pc += push::<4>(&mut runtime.stack, &bytecode[pc + 1..]),
            OpCode::PUSH5 => pc += push::<5>(&mut runtime.stack, &bytecode[pc + 1..]),
            OpCode::PUSH6 => pc += push::<6>(&mut runtime.stack, &bytecode[pc + 1..]),
            OpCode::PUSH7 => pc += push::<7>(&mut runtime.stack, &bytecode[pc + 1..]),
            OpCode::PUSH8 => pc += push::<8>(&mut runtime.stack, &bytecode[pc + 1..]),
            OpCode::PUSH9 => pc += push::<9>(&mut runtime.stack, &bytecode[pc + 1..]),
            OpCode::PUSH10 => pc += push::<10>(&mut runtime.stack, &bytecode[pc + 1..]),
            OpCode::PUSH11 => pc += push::<11>(&mut runtime.stack, &bytecode[pc + 1..]),
            OpCode::PUSH12 => pc += push::<12>(&mut runtime.stack, &bytecode[pc + 1..]),
            OpCode::PUSH13 => pc += push::<13>(&mut runtime.stack, &bytecode[pc + 1..]),
            OpCode::PUSH14 => pc += push::<14>(&mut runtime.stack, &bytecode[pc + 1..]),
            OpCode::PUSH15 => pc += push::<15>(&mut runtime.stack, &bytecode[pc + 1..]),
            OpCode::PUSH16 => pc += push::<16>(&mut runtime.stack, &bytecode[pc + 1..]),
            OpCode::PUSH17 => pc += push::<17>(&mut runtime.stack, &bytecode[pc + 1..]),
            OpCode::PUSH18 => pc += push::<18>(&mut runtime.stack, &bytecode[pc + 1..]),
            OpCode::PUSH19 => pc += push::<19>(&mut runtime.stack, &bytecode[pc + 1..]),
            OpCode::PUSH20 => pc += push::<20>(&mut runtime.stack, &bytecode[pc + 1..]),
            OpCode::PUSH21 => pc += push::<21>(&mut runtime.stack, &bytecode[pc + 1..]),
            OpCode::PUSH22 => pc += push::<22>(&mut runtime.stack, &bytecode[pc + 1..]),
            OpCode::PUSH23 => pc += push::<23>(&mut runtime.stack, &bytecode[pc + 1..]),
            OpCode::PUSH24 => pc += push::<24>(&mut runtime.stack, &bytecode[pc + 1..]),
            OpCode::PUSH25 => pc += push::<25>(&mut runtime.stack, &bytecode[pc + 1..]),
            OpCode::PUSH26 => pc += push::<26>(&mut runtime.stack, &bytecode[pc + 1..]),
            OpCode::PUSH27 => pc += push::<27>(&mut runtime.stack, &bytecode[pc + 1..]),
            OpCode::PUSH28 => pc += push::<28>(&mut runtime.stack, &bytecode[pc + 1..]),
            OpCode::PUSH29 => pc += push::<29>(&mut runtime.stack, &bytecode[pc + 1..]),
            OpCode::PUSH30 => pc += push::<30>(&mut runtime.stack, &bytecode[pc + 1..]),
            OpCode::PUSH31 => pc += push::<31>(&mut runtime.stack, &bytecode[pc + 1..]),
            OpCode::PUSH32 => pc += push::<32>(&mut runtime.stack, &bytecode[pc + 1..]),
            OpCode::DUP1 => dup::<1>(&mut runtime.stack),
            OpCode::DUP2 => dup::<2>(&mut runtime.stack),
            OpCode::DUP3 => dup::<3>(&mut runtime.stack),
            OpCode::DUP4 => dup::<4>(&mut runtime.stack),
            OpCode::DUP5 => dup::<5>(&mut runtime.stack),
            OpCode::DUP6 => dup::<6>(&mut runtime.stack),
            OpCode::DUP7 => dup::<7>(&mut runtime.stack),
            OpCode::DUP8 => dup::<8>(&mut runtime.stack),
            OpCode::DUP9 => dup::<9>(&mut runtime.stack),
            OpCode::DUP10 => dup::<10>(&mut runtime.stack),
            OpCode::DUP11 => dup::<11>(&mut runtime.stack),
            OpCode::DUP12 => dup::<12>(&mut runtime.stack),
            OpCode::DUP13 => dup::<13>(&mut runtime.stack),
            OpCode::DUP14 => dup::<14>(&mut runtime.stack),
            OpCode::DUP15 => dup::<15>(&mut runtime.stack),
            OpCode::DUP16 => dup::<16>(&mut runtime.stack),
            OpCode::SWAP1 => swap::<1>(&mut runtime.stack),
            OpCode::SWAP2 => swap::<2>(&mut runtime.stack),
            OpCode::SWAP3 => swap::<3>(&mut runtime.stack),
            OpCode::SWAP4 => swap::<4>(&mut runtime.stack),
            OpCode::SWAP5 => swap::<5>(&mut runtime.stack),
            OpCode::SWAP6 => swap::<6>(&mut runtime.stack),
            OpCode::SWAP7 => swap::<7>(&mut runtime.stack),
            OpCode::SWAP8 => swap::<8>(&mut runtime.stack),
            OpCode::SWAP9 => swap::<9>(&mut runtime.stack),
            OpCode::SWAP10 => swap::<10>(&mut runtime.stack),
            OpCode::SWAP11 => swap::<11>(&mut runtime.stack),
            OpCode::SWAP12 => swap::<12>(&mut runtime.stack),
            OpCode::SWAP13 => swap::<13>(&mut runtime.stack),
            OpCode::SWAP14 => swap::<14>(&mut runtime.stack),
            OpCode::SWAP15 => swap::<15>(&mut runtime.stack),
            OpCode::SWAP16 => swap::<16>(&mut runtime.stack),
            OpCode::LOG0 => log(runtime, system, 0)?,
            OpCode::LOG1 => log(runtime, system, 1)?,
            OpCode::LOG2 => log(runtime, system, 2)?,
            OpCode::LOG3 => log(runtime, system, 3)?,
            OpCode::LOG4 => log(runtime, system, 4)?,
            OpCode::CREATE => storage::create(runtime, system, false)?,
            OpCode::CREATE2 => storage::create(runtime, system, true)?,
            OpCode::CALL => call::call(runtime, system, CallKind::Call, false)?,
            OpCode::CALLCODE => call::call(runtime, system, CallKind::CallCode, false)?,
            OpCode::DELEGATECALL => call::call(runtime, system, CallKind::DelegateCall, false)?,
            OpCode::STATICCALL => call::call(runtime, system, CallKind::Call, true)?,
            OpCode::RETURN | OpCode::REVERT => {
                control::ret(runtime)?;
                reverted = op == OpCode::REVERT;
                break;
            }
            OpCode::INVALID => return Err(StatusCode::InvalidInstruction),
            OpCode::SELFDESTRUCT => storage::selfdestruct(runtime, system)?,
        }

        pc += 1; // advance
    }

    Ok(Output {
        reverted,
        status_code: StatusCode::Success,
        output_data: runtime.output_data.clone(),
    })
}
