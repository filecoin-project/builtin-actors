#![allow(clippy::unnecessary_mut_passed)]

pub mod arithmetic;
pub mod bitwise;
pub mod boolean;
pub mod call;
pub mod context;
pub mod control;
pub mod ext;
pub mod hash;
pub mod lifecycle;
pub mod log;
pub mod memory;
pub mod stack;
pub mod state;
pub mod storage;

use crate::interpreter::execution::Machine;
use crate::interpreter::output::StatusCode;
use crate::interpreter::U256;
use fil_actors_runtime::runtime::Runtime;

macro_rules! rev {
    ($($args:ident),*) => {
        rev!(() $($args),*)
    };
    (($($reversed:ident),*) $first:ident $(, $rest:ident)*) => {
        rev!(($first $(,$reversed)*) $($rest),*)
    };
    (($($reversed:ident),*)) => {
        [$($reversed),*]
    };
}

macro_rules! def_op {
    ($op:ident ($m:ident) => { $($body:tt)* }) => {
        #[allow(non_snake_case)]
        #[inline(always)]
        pub fn $op<'r, 'a, RT: Runtime + 'a>($m: &mut Machine<'r, 'a, RT> ) -> Result<(), StatusCode> {
            $($body)*
        }
    }
}

// macros for the instruction zoo:
// primops: take values of the stack and return a result value to be pushed on the stack
macro_rules! def_primop {
    ($op:ident ($($arg:ident),+) => $impl:path) => {
        def_op!{ $op (m) => {
            let &rev![$($arg),*] = m.state.stack.pop_many()?;
            let result = $impl($($arg),*);
            m.state.stack.push_unchecked(result);
            m.pc += 1;
            Ok(())

        }}
    };
    ($op:ident () => $impl:path) => {
        def_op!{ $op (m) => {
            m.state.stack.ensure_one();
            let result = $impl($($arg),*);
            m.state.stack.push_unchecked(result);
            m.pc += 1;
            Ok(())
        }}
    };
}

// stackops: operate directly on the stack
macro_rules! def_stackop {
    ($op:ident => $impl:path) => {
        def_op! { $op (m) => {
            $impl(&mut m.state.stack)?;
            m.pc += 1;
            Ok(())
        }}
    };
}

// pusho variants: push stuff on the stack taken as input from bytecode; the kind of thing that
// makes you want to cry because it really is a stack op.
macro_rules! def_push {
    ($op:ident => $impl:path) => {
        def_op! { $op (m) => {
            m.pc += 1;
            let code = &m.bytecode[m.pc..];
            m.pc += $impl(&mut m.state.stack, code)?;
            Ok(())
        }}
    };
}

// stdfuns: take state and system as first args, and args from the stack and return a result value
// to be pushed in the stack.
macro_rules! def_stdfun {
    ($op:ident ($($arg:ident),+) => $impl:path) => {
        def_op!{ $op (m) => {
            let &rev![$($arg),*] = m.state.stack.pop_many()?;
            let result = $impl(&mut m.state, &mut m.system, $($arg),*)?;
            m.state.stack.push_unchecked(result);
            m.pc += 1;
            Ok(())
        }}
    };
    ($op:ident () => $impl:path) => {
        def_op!{ $op (m) => {
            m.state.stack.ensure_one()?;
            let result = $impl(&mut m.state, &mut m.system)?;
            m.state.stack.push_unchecked(result);
            m.pc += 1;
            Ok(())
        }}
    };
}

// stdproc: like stdfun, but returns no value
macro_rules! def_stdproc {
    ($op:ident ($($arg:ident),*) => $impl:path) => {
        def_op!{ $op (m) => {
            let &rev![$($arg),*] = m.state.stack.pop_many()?;
            $impl(&mut m.state, &mut m.system, $($arg),*)?;
            m.pc += 1;
            Ok(())
        }}
    }
}

// std*_code: code reflective functionoid
macro_rules! def_stdfun_code {
    ($op:ident ($($arg:ident),+) => $impl:path) => {
        def_op!{ $op (m) => {
            let &rev![$($arg),*] = m.state.stack.pop_many()?;
            let result = $impl(&mut m.state, &mut m.system, m.bytecode.as_ref(), $($arg),*)?;
            m.state.stack.push_unchecked(result);
            m.pc += 1;
            Ok(())
        }}
    };
    ($op:ident () => $impl:path) => {
        def_op!{ $op (m) => {
            m.state.stack.ensure_one()?;
            let result = $impl(&mut m.state, &mut m.system, m.bytecode.as_ref())?;
            m.state.stack.push_unchecked(result);
            m.pc += 1;
            Ok(())
        }}
    };
}

// and the procedural variant
macro_rules! def_stdproc_code {
    ($op:ident ($($arg:ident),*) => $impl:path) => {
        def_op!{ $op (m) => {
            let &rev![$($arg),*] = m.state.stack.pop_many()?;
            $impl(&mut m.state, &mut m.system, m.bytecode.as_ref(), $($arg),*)?;
            m.pc += 1;
            Ok(())
        }}
    }
}

// stdproc: logging functionoid
macro_rules! def_stdlog {
    ($op:ident ($ntopics:literal, ($($topic:ident),*))) => {
        def_op!{ $op (m) => {
            let &rev![a, b $(,$topic)*] = m.state.stack.pop_many()?;
            log::log(&mut m.state, &mut m.system, $ntopics, a, b, &[$($topic),*])?;
            m.pc += 1;
            Ok(())
        }}
    }
}

// jmp: jump variants
macro_rules! def_jmp {
    ($op:ident ($($arg:ident),*) => $impl:path) => {
        def_op!{ $op (m) => {
            let &rev![$($arg),*] = m.state.stack.pop_many()?;
            m.pc = $impl(m.bytecode, m.pc, $($arg),*)?;
            Ok(())
        }}
    }

}
macro_rules! def_exit {
    ($op:ident ($($arg:ident),*) => $impl:path) => {
        def_op!{ $op (m) => {
            let &rev![$($arg),*] = m.state.stack.pop_many()?;
            m.output = $impl(&mut m.state, &mut m.system, $($arg),*)?;
            m.pc = m.bytecode.len(); // stop execution
            Ok(())
        }}
    }
}

// special: pc and things like that
macro_rules! def_special {
    ($op:ident ($m:ident) => $value:expr) => {
        def_op! { $op (m) => {
            let result = {
                let $m = &mut *m;
                $value
            };
            m.state.stack.push(result)?;
            m.pc += 1;
            Ok(())
        }}
    };
}

// IMPLEMENTATION
// arithmetic
def_primop! { ADD(a, b) => arithmetic::add }
def_primop! { MUL(a, b) => arithmetic::mul }
def_primop! { SUB(a, b) => arithmetic::sub }
def_primop! { DIV(a, b) => arithmetic::div }
def_primop! { SDIV(a, b) => arithmetic::sdiv }
def_primop! { MOD(a, b) => arithmetic::modulo }
def_primop! { SMOD(a, b) => arithmetic::smod }
def_primop! { ADDMOD(a, b, c) => arithmetic::addmod }
def_primop! { MULMOD(a, b, c) => arithmetic::mulmod }
def_primop! { EXP(a, b) => arithmetic::exp }
def_primop! { SIGNEXTEND(a, b) => arithmetic::signextend }
// boolean
def_primop! { LT(a, b) => boolean::lt }
def_primop! { GT(a, b) => boolean::gt }
def_primop! { SLT(a, b) => boolean::slt }
def_primop! { SGT(a, b) => boolean::sgt }
def_primop! { EQ(a, b) => boolean::eq }
def_primop! { ISZERO(a) => boolean::iszero }
def_primop! { AND(a, b) => boolean::and }
def_primop! { OR(a, b) => boolean::or }
def_primop! { XOR(a, b) => boolean::xor }
def_primop! { NOT(a) => boolean::not }
// bitwise
def_primop! { BYTE(a, b) => bitwise::byte }
def_primop! { SHL(a, b) => bitwise::shl }
def_primop! { SHR(a, b) => bitwise::shr }
def_primop! { SAR(a, b) => bitwise::sar }
// dup
def_stackop! { DUP1 => stack::dup::<1> }
def_stackop! { DUP2 => stack::dup::<2> }
def_stackop! { DUP3 => stack::dup::<3> }
def_stackop! { DUP4 => stack::dup::<4> }
def_stackop! { DUP5 => stack::dup::<5> }
def_stackop! { DUP6 => stack::dup::<6> }
def_stackop! { DUP7 => stack::dup::<7> }
def_stackop! { DUP8 => stack::dup::<8> }
def_stackop! { DUP9 => stack::dup::<9> }
def_stackop! { DUP10 => stack::dup::<10> }
def_stackop! { DUP11 => stack::dup::<11> }
def_stackop! { DUP12 => stack::dup::<12> }
def_stackop! { DUP13 => stack::dup::<13> }
def_stackop! { DUP14 => stack::dup::<14> }
def_stackop! { DUP15 => stack::dup::<15> }
def_stackop! { DUP16 => stack::dup::<16> }
// swap
def_stackop! { SWAP1 => stack::swap::<1> }
def_stackop! { SWAP2 => stack::swap::<2> }
def_stackop! { SWAP3 => stack::swap::<3> }
def_stackop! { SWAP4 => stack::swap::<4> }
def_stackop! { SWAP5 => stack::swap::<5> }
def_stackop! { SWAP6 => stack::swap::<6> }
def_stackop! { SWAP7 => stack::swap::<7> }
def_stackop! { SWAP8 => stack::swap::<8> }
def_stackop! { SWAP9 => stack::swap::<9> }
def_stackop! { SWAP10 => stack::swap::<10> }
def_stackop! { SWAP11 => stack::swap::<11> }
def_stackop! { SWAP12 => stack::swap::<12> }
def_stackop! { SWAP13 => stack::swap::<13> }
def_stackop! { SWAP14 => stack::swap::<14> }
def_stackop! { SWAP15 => stack::swap::<15> }
def_stackop! { SWAP16 => stack::swap::<16> }
// pop
def_stackop! { POP => stack::pop }
// push
def_push! { PUSH1 => stack::push::<1> }
def_push! { PUSH2 => stack::push::<2> }
def_push! { PUSH3 => stack::push::<3> }
def_push! { PUSH4 => stack::push::<4> }
def_push! { PUSH5 => stack::push::<5> }
def_push! { PUSH6 => stack::push::<6> }
def_push! { PUSH7 => stack::push::<7> }
def_push! { PUSH8 => stack::push::<8> }
def_push! { PUSH9 => stack::push::<9> }
def_push! { PUSH10 => stack::push::<10> }
def_push! { PUSH11 => stack::push::<11> }
def_push! { PUSH12 => stack::push::<12> }
def_push! { PUSH13 => stack::push::<13> }
def_push! { PUSH14 => stack::push::<14> }
def_push! { PUSH15 => stack::push::<15> }
def_push! { PUSH16 => stack::push::<16> }
def_push! { PUSH17 => stack::push::<17> }
def_push! { PUSH18 => stack::push::<18> }
def_push! { PUSH19 => stack::push::<19> }
def_push! { PUSH20 => stack::push::<20> }
def_push! { PUSH21 => stack::push::<21> }
def_push! { PUSH22 => stack::push::<22> }
def_push! { PUSH23 => stack::push::<23> }
def_push! { PUSH24 => stack::push::<24> }
def_push! { PUSH25 => stack::push::<25> }
def_push! { PUSH26 => stack::push::<26> }
def_push! { PUSH27 => stack::push::<27> }
def_push! { PUSH28 => stack::push::<28> }
def_push! { PUSH29 => stack::push::<29> }
def_push! { PUSH30 => stack::push::<30> }
def_push! { PUSH31 => stack::push::<31> }
def_push! { PUSH32 => stack::push::<32> }
// functionoids
def_stdfun! { KECCAK256(a, b) => hash::keccak256 }
def_stdfun! { ADDRESS() => context::address }
def_stdfun! { BALANCE(a) => state::balance }
def_stdfun! { ORIGIN() => context::origin }
def_stdfun! { CALLER() => context::caller }
def_stdfun! { CALLVALUE() => context::call_value }
def_stdfun! { CALLDATALOAD(a) => call::calldataload }
def_stdfun! { CALLDATASIZE() => call::calldatasize }
def_stdproc! { CALLDATACOPY(a, b, c) => call::calldatacopy }
def_stdfun! { GASPRICE() => context::gas_price }
def_stdfun! { EXTCODESIZE(a) => ext::extcodesize }
def_stdproc! { EXTCODECOPY(a, b, c, d) => ext::extcodecopy }
def_stdfun! { EXTCODEHASH(a) => ext::extcodehash }
def_stdfun! { RETURNDATASIZE() => control::returndatasize }
def_stdproc! { RETURNDATACOPY(a, b, c) => control::returndatacopy }
def_stdfun! { BLOCKHASH(a) => context::blockhash }
def_stdfun! { COINBASE() => context::coinbase }
def_stdfun! { TIMESTAMP() => context::timestamp }
def_stdfun! { NUMBER() => context::block_number }
def_stdfun! { PREVRANDAO() => context::prevrandao }
def_stdfun! { GASLIMIT() => context::gas_limit }
def_stdfun! { CHAINID() => context::chain_id }
def_stdfun! { BASEFEE() => context::base_fee }
def_stdfun! { SELFBALANCE() => state::selfbalance }
def_stdfun! { MLOAD(a) => memory::mload }
def_stdproc! { MSTORE(a, b) => memory::mstore }
def_stdproc! { MSTORE8(a, b) => memory::mstore8 }
def_stdfun! { SLOAD(a) => storage::sload }
def_stdproc! { SSTORE(a, b) => storage::sstore }
def_stdfun! { MSIZE() => memory::msize }
def_stdfun! { GAS() => context::gas }
def_stdlog! { LOG0(0, ()) }
def_stdlog! { LOG1(1, (topic1)) }
def_stdlog! { LOG2(2, (topic1, topic2)) }
def_stdlog! { LOG3(3, (topic1, topic2, topic3)) }
def_stdlog! { LOG4(4, (topic1, topic2, topic3, topic4)) }
def_stdfun! { CALL(gas, dst, value, ioff, isz, ooff, osz) => call::call_call }
def_stdfun! { CALLCODE(gas, dst, value, ioff, isz, ooff, osz) => call::call_callcode }
def_stdfun! { DELEGATECALL(gas, dst, ioff, isz, ooff, osz) => call::call_delegatecall }
def_stdfun! { STATICCALL(gas, dst, ioff, isz, ooff, osz) => call::call_staticcall }
def_stdfun_code! { CODESIZE() => call::codesize }
def_stdproc_code! { CODECOPY(a, b, c) => call::codecopy }
def_stdfun! { CREATE(a, b, c) => lifecycle::create }
def_stdfun! { CREATE2(a, b, c, d) => lifecycle::create2 }
def_stdproc! { JUMPDEST() => control::nop }
def_stdproc! { INVALID() => control::invalid }
def_exit! { RETURN(a, b) => control::ret }
def_exit! { REVERT(a, b) => control::revert }
def_exit! { STOP() => control::stop }
def_exit! { SELFDESTRUCT(a) => lifecycle::selfdestruct }
def_jmp! { JUMP(a) => control::jump }
def_jmp! { JUMPI(a, b) => control::jumpi }
def_special! { PC(m) => U256::from(m.pc) }
