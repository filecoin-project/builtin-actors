//! EVM Opcodes as of Berlin Hard Fork
//!
//! On filecoin we will never have to replay blocks that are older
//! than the release date of the FVM-EVM runtime, so supporting
//! historic behavior is not needed.

use crate::interpreter::output::StatusCode;

macro_rules! def_opcodes {
    ($($code:literal: $name:ident($stack:literal, $change:literal),)*) => {
        #[repr(u8)]
        #[derive(Copy, Clone, Debug, PartialEq, Eq)]
        pub enum OpCode {
            $($name = $code,)*
        }
        #[derive(Copy, Clone, Debug)]
        pub struct StackSpec {
            pub required: u8,
            pub changed: i8,
        }

        impl std::fmt::Display for OpCode {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str(self.name())
            }
        }

        impl From<OpCode> for u8 {
            #[inline(always)]
            fn from(op: OpCode) -> Self {
                op as u8
            }
        }

        impl TryFrom<u8> for OpCode {
            type Error = StatusCode;

            fn try_from(op: u8) -> Result<Self, Self::Error> {
                const fn codes() -> [bool; 256] {
                    let mut table = [false; 256];
                    $(table[$code] = true;)*
                    table
                }
                const CODES: [bool; 256] = codes();
                if !CODES[op as usize] {
                    return Err(StatusCode::UndefinedInstruction);
                }

                Ok(unsafe { std::mem::transmute(op) })
            }
        }

        impl PartialEq<u8> for OpCode {
            fn eq(&self, other: &u8) -> bool {
                (*self as u8) == *other
            }
        }

        impl OpCode {
            pub const fn spec(self) -> StackSpec {
                const fn specs() -> [StackSpec; 256] {
                    let mut table = [StackSpec{required: 0, changed: 0}; 256];
                    $(table[$code] = StackSpec{required: $stack, changed: $change};)*
                    table
                }
                const SPECS: [StackSpec; 256] = specs();
                SPECS[self as usize]
            }
            pub const fn name(self) -> &'static str {
                const fn names() -> [&'static str; 256] {
                    let mut table = ["RESERVED"; 256];
                    $(table[$code] = stringify!($name);)*
                    table
                }
                const NAMES: [&'static str; 256] = names();
                NAMES[self as usize]
            }
        }
    }
}

def_opcodes! {
    0x00: STOP(0, 0),
    0x01: ADD(2, -1),
    0x02: MUL(2, -1),
    0x03: SUB(2, -1),
    0x04: DIV(2, -1),
    0x05: SDIV(2, -1),
    0x06: MOD(2, -1),
    0x07: SMOD(2, -1),
    0x08: ADDMOD(3, -2),
    0x09: MULMOD(3, -2),
    0x0a: EXP(2, -1),
    0x0b: SIGNEXTEND(2, -1),
    0x10: LT(2, -1),
    0x11: GT(2, -1),
    0x12: SLT(2, -1),
    0x13: SGT(2, -1),
    0x14: EQ(2, -1),
    0x15: ISZERO(1, 0),
    0x16: AND(2, -1),
    0x17: OR(2, -1),
    0x18: XOR(2, -1),
    0x19: NOT(1, 0),
    0x1a: BYTE(2, -1),
    0x1b: SHL(2, -1),
    0x1c: SHR(2, -1),
    0x1d: SAR(2, -1),
    0x20: KECCAK256(2, -1), // SHA3
    0x30: ADDRESS(0, 1),
    0x31: BALANCE(1, 0),
    0x32: ORIGIN(0, 1),
    0x33: CALLER(0, 1),
    0x34: CALLVALUE(0, 1),
    0x35: CALLDATALOAD(1, 0),
    0x36: CALLDATASIZE(0, 1),
    0x37: CALLDATACOPY(3, -3),
    0x38: CODESIZE(0, 1),
    0x39: CODECOPY(3, -3),
    0x3a: GASPRICE(0, 1),
    0x3b: EXTCODESIZE(1, 0),
    0x3c: EXTCODECOPY(4, -4),
    0x3d: RETURNDATASIZE(0, 1),
    0x3e: RETURNDATACOPY(3, -3),
    0x3f: EXTCODEHASH(1, 0),
    0x40: BLOCKHASH(1, 0),
    0x41: COINBASE(0, 1),
    0x42: TIMESTAMP(0, 1),
    0x43: NUMBER(0, 1),
    0x44: DIFFICULTY(0, 1),
    0x45: GASLIMIT(0, 1),
    0x46: CHAINID(0, 1),
    0x47: SELFBALANCE(0, 1),
    0x48: BASEFEE(0, 1),
    0x50: POP(1, -1),
    0x51: MLOAD(1, 0),
    0x52: MSTORE(2, -2),
    0x53: MSTORE8(2, -2),
    0x54: SLOAD(1, 0),
    0x55: SSTORE(2, -2),
    0x56: JUMP(1, -1),
    0x57: JUMPI(2, -2),
    0x58: PC(0, 1),
    0x59: MSIZE(0, 1),
    0x5a: GAS(0, 1),
    0x5b: JUMPDEST(0, 0),
    0x60: PUSH1(0, 1),
    0x61: PUSH2(0, 1),
    0x62: PUSH3(0, 1),
    0x63: PUSH4(0, 1),
    0x64: PUSH5(0, 1),
    0x65: PUSH6(0, 1),
    0x66: PUSH7(0, 1),
    0x67: PUSH8(0, 1),
    0x68: PUSH9(0, 1),
    0x69: PUSH10(0, 1),
    0x6a: PUSH11(0, 1),
    0x6b: PUSH12(0, 1),
    0x6c: PUSH13(0, 1),
    0x6d: PUSH14(0, 1),
    0x6e: PUSH15(0, 1),
    0x6f: PUSH16(0, 1),
    0x70: PUSH17(0, 1),
    0x71: PUSH18(0, 1),
    0x72: PUSH19(0, 1),
    0x73: PUSH20(0, 1),
    0x74: PUSH21(0, 1),
    0x75: PUSH22(0, 1),
    0x76: PUSH23(0, 1),
    0x77: PUSH24(0, 1),
    0x78: PUSH25(0, 1),
    0x79: PUSH26(0, 1),
    0x7a: PUSH27(0, 1),
    0x7b: PUSH28(0, 1),
    0x7c: PUSH29(0, 1),
    0x7d: PUSH30(0, 1),
    0x7e: PUSH31(0, 1),
    0x7f: PUSH32(0, 1),
    0x80: DUP1(1, 1),
    0x81: DUP2(2, 1),
    0x82: DUP3(3, 1),
    0x83: DUP4(4, 1),
    0x84: DUP5(5, 1),
    0x85: DUP6(6, 1),
    0x86: DUP7(7, 1),
    0x87: DUP8(8, 1),
    0x88: DUP9(9, 1),
    0x89: DUP10(10, 1),
    0x8a: DUP11(11, 1),
    0x8b: DUP12(12, 1),
    0x8c: DUP13(13, 1),
    0x8d: DUP14(14, 1),
    0x8e: DUP15(15, 1),
    0x8f: DUP16(16, 1),
    0x90: SWAP1(2, 0),
    0x91: SWAP2(3, 0),
    0x92: SWAP3(4, 0),
    0x93: SWAP4(5, 0),
    0x94: SWAP5(6, 0),
    0x95: SWAP6(7, 0),
    0x96: SWAP7(8, 0),
    0x97: SWAP8(9, 0),
    0x98: SWAP9(10, 0),
    0x99: SWAP10(11, 0),
    0x9a: SWAP11(12, 0),
    0x9b: SWAP12(13, 0),
    0x9c: SWAP13(14, 0),
    0x9d: SWAP14(15, 0),
    0x9e: SWAP15(16, 0),
    0x9f: SWAP16(17, 0),
    0xa0: LOG0(2, -2),
    0xa1: LOG1(3, -3),
    0xa2: LOG2(4, -4),
    0xa3: LOG3(5, -5),
    0xa4: LOG4(6, -6),
    // 0xEF Reserved for EIP-3541
    0xf0: CREATE(3, -2),
    0xf1: CALL(7, -6),
    0xf2: CALLCODE(7, -6),
    0xf3: RETURN(2, -2),
    0xf4: DELEGATECALL(6, -5),
    0xf5: CREATE2(4, -3),
    0xfa: STATICCALL(6, -5),
    0xfd: REVERT(2, -2),
    0xfe: INVALID(0, 0),
    0xff: SELFDESTRUCT(1, -1),
}
