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

// macros
macro_rules! def_primop {
    ($op:ident ($($arg:ident),*) => $impl:path) => {
        #[allow(non_snake_case)]
        pub fn $op(sk: &mut Stack) -> Result<(), StatusCode> {
            $(let $arg = sk.pop();)*
            let result = $impl($($arg),*);
            sk.push(result);
            Ok(())
        }
    }
}

// CHECK+DISPATCH
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
