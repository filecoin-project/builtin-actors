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

use crate::interpreter::stack::Stack;
use crate::interpreter::output::StatusCode;
use crate::interpreter::opcode::{OpCode,StackSpec};
use crate::interpreter::{ExecutionState, System};
use fil_actors_runtime::runtime::Runtime;

// macros for the instruction zoo:
// primops: take values of the stack and return a result value to be pushed on the stack
macro_rules! def_primop {
    ($op:ident ($($arg:ident),*) => $impl:path) => {
        #[allow(non_snake_case)]
        pub fn $op(sk: &mut Stack) -> Result<(), StatusCode> {
            check_arity!($op, ($($arg),*));
            check_stack!($op, sk);
            $(let $arg = sk.pop();)*
            let result = $impl($($arg),*);
            sk.push(result);
            Ok(())
        }
    }
}

// stackops: operate directly on the stack
macro_rules! def_stackop {
    ($op:ident => $impl:path) => {
        #[allow(non_snake_case)]
        pub fn $op(sk: &mut Stack) -> Result<(), StatusCode> {
            check_stack!($op, sk);
            $impl(sk);
            Ok(())
        }
    }
}

// pushops: push stuff on the stack given input bytecode; the kind of thing that makes you want
// to cry because it really is a stack op.
macro_rules! def_push {
    ($op:ident => $impl:path) => {
        #[allow(non_snake_case)]
        pub fn $op(sk: &mut Stack, code: &[u8]) -> Result<usize, StatusCode> {
            check_stack!($op, sk);
            let off = $impl(sk, code);
            Ok(off)
        }
    }
}

// stdfuns: take state and system as first args, and args from the stack and return a result value
// to be pushed in the stack.
macro_rules! def_stdfun {
    ($op:ident ($($arg:ident),*) => $impl:path) => {
        #[allow(non_snake_case)]
        pub fn $op(state: &mut ExecutionState, system: &System<impl Runtime>) -> Result<(), StatusCode> {
            check_arity!($op, ($($arg),*));
            check_stack!($op, state.stack);
            $(let $arg = state.stack.pop();)*
            let result = $impl(state, system, $($arg),*)?;
            state.stack.push(result);
            Ok(())
        }
    }
}

// stdproc: like stdfun, but returns no value
macro_rules! def_stdproc {
    ($op:ident ($($arg:ident),*) => $impl:path) => {
        #[allow(non_snake_case)]
        pub fn $op(state: &mut ExecutionState, system: &System<impl Runtime>) -> Result<(), StatusCode> {
            check_arity!($op, ($($arg),*));
            check_stack!($op, state.stack);
            $(let $arg = state.stack.pop();)*
            $impl(state, system, $($arg),*)?;
            Ok(())
        }
    }
}

// auxiliary macros
macro_rules! check_stack {
    ($op:ident, $sk:expr) => {{
        const SPEC: StackSpec = OpCode::$op.spec();
        if SPEC.required > 0 {
            if !$sk.require(SPEC.required as usize) {
                return Err(StatusCode::StackUnderflow);
            }
        }
        if SPEC.changed > 0 {
            if !$sk.ensure(SPEC.changed as usize) {
                return Err(StatusCode::StackOverflow);
            }
        }
    }};
}

macro_rules! check_arity {
    ($op:ident, ($($arg:ident),*)) => {{
        #[allow(dead_code)]
        const fn checkargs() {
            const SPEC: StackSpec = OpCode::$op.spec();
            // the error message is super ugly, but this static asserts we got the
            // arity of the primop right.
            const _: [();(arg_count!($($arg),*)) - SPEC.required as usize] = [];
        }
        checkargs();
    }}
}

macro_rules! arg_count {
    () => {0};
    ($arg:ident) => {1};
    ($arg:ident, $arg2:ident) => {2};
    ($arg:ident, $arg2:ident, $arg3:ident) => {3};
    // can't use this coz we need a literal number
    //($arg: ident, $($rest:ident),*) => { 1 + arg_count!($($rest),*) };
}


// IMPLEMENTATION
// arithmetic
def_primop!{ ADD(a, b) => arithmetic::add }
def_primop!{ MUL(a, b) => arithmetic::mul }
def_primop!{ SUB(a, b) => arithmetic::sub }
def_primop!{ DIV(a, b) => arithmetic::div }
def_primop!{ SDIV(a, b) => arithmetic::sdiv }
def_primop!{ MOD(a, b) => arithmetic::modulo }
def_primop!{ SMOD(a, b) => arithmetic::smod }
def_primop!{ ADDMOD(a, b, c) => arithmetic::addmod }
def_primop!{ MULMOD(a, b, c) => arithmetic::mulmod }
def_primop!{ EXP(a, b) => arithmetic::exp }
def_primop!{ SIGNEXTEND(a, b) => arithmetic::signextend }
// boolean
def_primop!{ LT(a, b) => boolean::lt }
def_primop!{ GT(a, b) => boolean::gt }
def_primop!{ SLT(a, b) => boolean::slt }
def_primop!{ SGT(a, b) => boolean::sgt }
def_primop!{ EQ(a, b) => boolean::eq }
def_primop!{ ISZERO(a) => boolean::iszero }
def_primop!{ AND(a, b) => boolean::and }
def_primop!{ OR(a, b) => boolean::or }
def_primop!{ XOR(a, b) => boolean::xor }
def_primop!{ NOT(a) => boolean::not }
// bitwise
def_primop!{ BYTE(a, b) => bitwise::byte }
def_primop!{ SHL(a, b) => bitwise::shl }
def_primop!{ SHR(a, b) => bitwise::shr }
def_primop!{ SAR(a, b) => bitwise::sar }
// dup
def_stackop!{ DUP1 => stack::dup::<1> }
def_stackop!{ DUP2 => stack::dup::<2> }
def_stackop!{ DUP3 => stack::dup::<3> }
def_stackop!{ DUP4 => stack::dup::<4> }
def_stackop!{ DUP5 => stack::dup::<5> }
def_stackop!{ DUP6 => stack::dup::<6> }
def_stackop!{ DUP7 => stack::dup::<7> }
def_stackop!{ DUP8 => stack::dup::<8> }
def_stackop!{ DUP9 => stack::dup::<9> }
def_stackop!{ DUP10 => stack::dup::<10> }
def_stackop!{ DUP11 => stack::dup::<11> }
def_stackop!{ DUP12 => stack::dup::<12> }
def_stackop!{ DUP13 => stack::dup::<13> }
def_stackop!{ DUP14 => stack::dup::<14> }
def_stackop!{ DUP15 => stack::dup::<15> }
def_stackop!{ DUP16 => stack::dup::<16> }
// swap
def_stackop!{ SWAP1 => stack::swap::<1> }
def_stackop!{ SWAP2 => stack::swap::<2> }
def_stackop!{ SWAP3 => stack::swap::<3> }
def_stackop!{ SWAP4 => stack::swap::<4> }
def_stackop!{ SWAP5 => stack::swap::<5> }
def_stackop!{ SWAP6 => stack::swap::<6> }
def_stackop!{ SWAP7 => stack::swap::<7> }
def_stackop!{ SWAP8 => stack::swap::<8> }
def_stackop!{ SWAP9 => stack::swap::<9> }
def_stackop!{ SWAP10 => stack::swap::<10> }
def_stackop!{ SWAP11 => stack::swap::<11> }
def_stackop!{ SWAP12 => stack::swap::<12> }
def_stackop!{ SWAP13 => stack::swap::<13> }
def_stackop!{ SWAP14 => stack::swap::<14> }
def_stackop!{ SWAP15 => stack::swap::<15> }
def_stackop!{ SWAP16 => stack::swap::<16> }
// pop
def_stackop!{ POP => stack::pop }
// push
def_push!{ PUSH1 => stack::push::<1> }
def_push!{ PUSH2 => stack::push::<2> }
def_push!{ PUSH3 => stack::push::<3> }
def_push!{ PUSH4 => stack::push::<4> }
def_push!{ PUSH5 => stack::push::<5> }
def_push!{ PUSH6 => stack::push::<6> }
def_push!{ PUSH7 => stack::push::<7> }
def_push!{ PUSH8 => stack::push::<8> }
def_push!{ PUSH9 => stack::push::<9> }
def_push!{ PUSH10 => stack::push::<10> }
def_push!{ PUSH11 => stack::push::<11> }
def_push!{ PUSH12 => stack::push::<12> }
def_push!{ PUSH13 => stack::push::<13> }
def_push!{ PUSH14 => stack::push::<14> }
def_push!{ PUSH15 => stack::push::<15> }
def_push!{ PUSH16 => stack::push::<16> }
def_push!{ PUSH17 => stack::push::<17> }
def_push!{ PUSH18 => stack::push::<18> }
def_push!{ PUSH19 => stack::push::<19> }
def_push!{ PUSH20 => stack::push::<20> }
def_push!{ PUSH21 => stack::push::<21> }
def_push!{ PUSH22 => stack::push::<22> }
def_push!{ PUSH23 => stack::push::<23> }
def_push!{ PUSH24 => stack::push::<24> }
def_push!{ PUSH25 => stack::push::<25> }
def_push!{ PUSH26 => stack::push::<26> }
def_push!{ PUSH27 => stack::push::<27> }
def_push!{ PUSH28 => stack::push::<28> }
def_push!{ PUSH29 => stack::push::<29> }
def_push!{ PUSH30 => stack::push::<30> }
def_push!{ PUSH31 => stack::push::<31> }
def_push!{ PUSH32 => stack::push::<32> }
// stdfuns
def_stdfun!{ KECCAK256(a, b) => hash::keccak256 }
def_stdfun!{ ADDRESS() => context::address }
def_stdfun!{ BALANCE(a) => state::balance }
def_stdfun!{ ORIGIN() => context::origin }
def_stdfun!{ CALLER() => context::caller }
def_stdfun!{ CALLVALUE() => context::call_value }
def_stdfun!{ CALLDATALOAD(a) => call::calldataload }
def_stdfun!{ CALLDATASIZE() => call::calldatasize }
def_stdproc!{ CALLDATACOPY(a, b, c) => call::calldatacopy }
