//! EVM Opcodes as of Berlin Hard Fork
//!
//! On filecoin we will never have to replay blocks that are older
//! than the release date of the FVM-EVM runtime, so supporting
//! historic behavior is not needed.

use crate::interp::output::StatusCode;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct OpCode {
    /// the byte representing the opcode in binary
    pub code: u8,

    /// cost of executing the opcode, subtracted from the
    /// total gas limit when running bytecode.
    pub price: u16,

    /// The number of stack items the instruction accesses during execution.
    pub stack_height_required: u8,

    /// The stack height change caused by the instruction execution. Can be
    /// negative.
    pub stack_height_change: i8,

    /// Human readable name of the opcode.
    pub name: &'static str,
}

impl From<OpCode> for u8 {
    fn from(op: OpCode) -> Self {
        op.code
    }
}

impl PartialEq<u8> for OpCode {
    fn eq(&self, other: &u8) -> bool {
        self.code == *other
    }
}

const _COLD_SLOAD_COST: u16 = 2100;
const _COLD_ACCOUNT_ACCESS_COST: u16 = 2600;
const WARM_STORAGE_READ_COST: u16 = 100;

impl OpCode {
    pub const ADD: OpCode = OpCode {
        code: 0x01,
        price: 3,
        stack_height_required: 2,
        stack_height_change: -1,
        name: "ADD",
    };
    pub const ADDMOD: OpCode = OpCode {
        code: 0x08,
        price: 8,
        stack_height_required: 3,
        stack_height_change: -2,
        name: "ADDMOD",
    };
    pub const ADDRESS: OpCode = OpCode {
        code: 0x30,
        price: 2,
        stack_height_required: 0,
        stack_height_change: 1,
        name: "ADDRESS",
    };
    pub const AND: OpCode = OpCode {
        code: 0x16,
        price: 3,
        stack_height_required: 2,
        stack_height_change: -1,
        name: "AND",
    };
    pub const BALANCE: OpCode = OpCode {
        code: 0x31,
        price: WARM_STORAGE_READ_COST,
        stack_height_required: 1,
        stack_height_change: 0,
        name: "BALANCE",
    };
    pub const BASEFEE: OpCode = OpCode {
        code: 0x48,
        price: 2,
        stack_height_required: 0,
        stack_height_change: 1,
        name: "BASEFEE",
    };
    pub const BLOCKHASH: OpCode = OpCode {
        code: 0x40,
        price: 20,
        stack_height_required: 1,
        stack_height_change: 0,
        name: "BLOCKHASH",
    };
    pub const BYTE: OpCode = OpCode {
        code: 0x1a,
        price: 3,
        stack_height_required: 2,
        stack_height_change: -1,
        name: "BYTE",
    };
    pub const CALL: OpCode = OpCode {
        code: 0xf1,
        price: WARM_STORAGE_READ_COST,
        stack_height_required: 7,
        stack_height_change: -6,
        name: "CALL",
    };
    pub const CALLCODE: OpCode = OpCode {
        code: 0xf2,
        price: WARM_STORAGE_READ_COST,
        stack_height_required: 7,
        stack_height_change: -6,
        name: "CALLCODE",
    };
    pub const CALLDATACOPY: OpCode = OpCode {
        code: 0x37,
        price: 3,
        stack_height_required: 3,
        stack_height_change: -3,
        name: "CALLDATACOPY",
    };
    pub const CALLDATALOAD: OpCode = OpCode {
        code: 0x35,
        price: 3,
        stack_height_required: 1,
        stack_height_change: 0,
        name: "CALLDATALOAD",
    };
    pub const CALLDATASIZE: OpCode = OpCode {
        code: 0x36,
        price: 2,
        stack_height_required: 0,
        stack_height_change: 1,
        name: "CALLDATASIZE",
    };
    pub const CALLER: OpCode = OpCode {
        code: 0x33,
        price: 2,
        stack_height_required: 0,
        stack_height_change: 1,
        name: "CALLER",
    };
    pub const CALLVALUE: OpCode = OpCode {
        code: 0x34,
        price: 2,
        stack_height_required: 0,
        stack_height_change: 1,
        name: "CALLVALUE",
    };
    pub const CHAINID: OpCode = OpCode {
        code: 0x46,
        price: 2,
        stack_height_required: 0,
        stack_height_change: 1,
        name: "CHAINID",
    };
    pub const CODECOPY: OpCode = OpCode {
        code: 0x39,
        price: 3,
        stack_height_required: 3,
        stack_height_change: -3,
        name: "CODECOPY",
    };
    pub const CODESIZE: OpCode = OpCode {
        code: 0x38,
        price: 2,
        stack_height_required: 0,
        stack_height_change: 1,
        name: "CODESIZE",
    };
    pub const COINBASE: OpCode = OpCode {
        code: 0x41,
        price: 2,
        stack_height_required: 0,
        stack_height_change: 1,
        name: "COINBASE",
    };
    pub const CREATE: OpCode = OpCode {
        code: 0xf0,
        price: 32000,
        stack_height_required: 3,
        stack_height_change: -2,
        name: "CREATE",
    };
    pub const CREATE2: OpCode = OpCode {
        code: 0xf5,
        price: 32000,
        stack_height_required: 4,
        stack_height_change: -3,
        name: "CREATE2",
    };
    pub const DELEGATECALL: OpCode = OpCode {
        code: 0xf4,
        price: WARM_STORAGE_READ_COST,
        stack_height_required: 6,
        stack_height_change: -5,
        name: "DELEGATECALL",
    };
    pub const DIFFICULTY: OpCode = OpCode {
        code: 0x44,
        price: 2,
        stack_height_required: 0,
        stack_height_change: 1,
        name: "DIFFICULTY",
    };
    pub const DIV: OpCode = OpCode {
        code: 0x04,
        price: 5,
        stack_height_required: 2,
        stack_height_change: -1,
        name: "DIV",
    };
    pub const DUP1: OpCode = OpCode {
        code: 0x80,
        price: 3,
        stack_height_required: 1,
        stack_height_change: 1,
        name: "DUP1",
    };
    pub const DUP10: OpCode = OpCode {
        code: 0x89,
        price: 3,
        stack_height_required: 10,
        stack_height_change: 1,
        name: "DUP10",
    };
    pub const DUP11: OpCode = OpCode {
        code: 0x8a,
        price: 3,
        stack_height_required: 11,
        stack_height_change: 1,
        name: "DUP11",
    };
    pub const DUP12: OpCode = OpCode {
        code: 0x8b,
        price: 3,
        stack_height_required: 12,
        stack_height_change: 1,
        name: "DUP12",
    };
    pub const DUP13: OpCode = OpCode {
        code: 0x8c,
        price: 3,
        stack_height_required: 13,
        stack_height_change: 1,
        name: "DUP13",
    };
    pub const DUP14: OpCode = OpCode {
        code: 0x8d,
        price: 3,
        stack_height_required: 14,
        stack_height_change: 1,
        name: "DUP14",
    };
    pub const DUP15: OpCode = OpCode {
        code: 0x8e,
        price: 3,
        stack_height_required: 15,
        stack_height_change: 1,
        name: "DUP15",
    };
    pub const DUP16: OpCode = OpCode {
        code: 0x8f,
        price: 3,
        stack_height_required: 16,
        stack_height_change: 1,
        name: "DUP16",
    };
    pub const DUP2: OpCode = OpCode {
        code: 0x81,
        price: 3,
        stack_height_required: 2,
        stack_height_change: 1,
        name: "DUP2",
    };
    pub const DUP3: OpCode = OpCode {
        code: 0x82,
        price: 3,
        stack_height_required: 3,
        stack_height_change: 1,
        name: "DUP3",
    };
    pub const DUP4: OpCode = OpCode {
        code: 0x83,
        price: 3,
        stack_height_required: 4,
        stack_height_change: 1,
        name: "DUP4",
    };
    pub const DUP5: OpCode = OpCode {
        code: 0x84,
        price: 3,
        stack_height_required: 5,
        stack_height_change: 1,
        name: "DUP5",
    };
    pub const DUP6: OpCode = OpCode {
        code: 0x85,
        price: 3,
        stack_height_required: 6,
        stack_height_change: 1,
        name: "DUP6",
    };
    pub const DUP7: OpCode = OpCode {
        code: 0x86,
        price: 3,
        stack_height_required: 7,
        stack_height_change: 1,
        name: "DUP7",
    };
    pub const DUP8: OpCode = OpCode {
        code: 0x87,
        price: 3,
        stack_height_required: 8,
        stack_height_change: 1,
        name: "DUP8",
    };
    pub const DUP9: OpCode = OpCode {
        code: 0x88,
        price: 3,
        stack_height_required: 9,
        stack_height_change: 1,
        name: "DUP9",
    };
    pub const EQ: OpCode = OpCode {
        code: 0x14,
        price: 3,
        stack_height_required: 2,
        stack_height_change: -1,
        name: "EQ",
    };
    pub const EXP: OpCode = OpCode {
        code: 0x0a,
        price: 10,
        stack_height_required: 2,
        stack_height_change: -1,
        name: "EXP",
    };
    pub const EXTCODECOPY: OpCode = OpCode {
        code: 0x3c,
        price: WARM_STORAGE_READ_COST,
        stack_height_required: 4,
        stack_height_change: -4,
        name: "EXTCODECOPY",
    };
    pub const EXTCODEHASH: OpCode = OpCode {
        code: 0x3f,
        price: WARM_STORAGE_READ_COST,
        stack_height_required: 1,
        stack_height_change: 0,
        name: "EXTCODEHASH",
    };
    pub const EXTCODESIZE: OpCode = OpCode {
        code: 0x3b,
        price: WARM_STORAGE_READ_COST,
        stack_height_required: 1,
        stack_height_change: 0,
        name: "EXTCODESIZE",
    };
    pub const GAS: OpCode = OpCode {
        code: 0x5a,
        price: 2,
        stack_height_required: 0,
        stack_height_change: 1,
        name: "GAS",
    };
    pub const GASLIMIT: OpCode = OpCode {
        code: 0x45,
        price: 2,
        stack_height_required: 0,
        stack_height_change: 1,
        name: "GASLIMIT",
    };
    pub const GASPRICE: OpCode = OpCode {
        code: 0x3a,
        price: 2,
        stack_height_required: 0,
        stack_height_change: 1,
        name: "GASPRICE",
    };
    pub const GT: OpCode = OpCode {
        code: 0x11,
        price: 3,
        stack_height_required: 2,
        stack_height_change: -1,
        name: "GT",
    };
    pub const INVALID: OpCode = OpCode {
        code: 0xfe,
        price: 0,
        stack_height_required: 0,
        stack_height_change: 0,
        name: "INVALID",
    };
    pub const ISZERO: OpCode = OpCode {
        code: 0x15,
        price: 3,
        stack_height_required: 1,
        stack_height_change: 0,
        name: "ISZERO",
    };
    pub const JUMP: OpCode = OpCode {
        code: 0x56,
        price: 8,
        stack_height_required: 1,
        stack_height_change: -1,
        name: "JUMP",
    };
    pub const JUMPDEST: OpCode = OpCode {
        code: 0x5b,
        price: 1,
        stack_height_required: 0,
        stack_height_change: 0,
        name: "JUMPDEST",
    };
    pub const JUMPI: OpCode = OpCode {
        code: 0x57,
        price: 10,
        stack_height_required: 2,
        stack_height_change: -2,
        name: "JUMPI",
    };
    pub const KECCAK256: OpCode = OpCode {
        code: 0x20,
        price: 30,
        stack_height_required: 2,
        stack_height_change: -1,
        name: "KECCAK256",
    };
    pub const LOG0: OpCode = OpCode {
        code: 0xa0,
        price: 375,
        stack_height_required: 2,
        stack_height_change: -2,
        name: "LOG0",
    };
    pub const LOG1: OpCode = OpCode {
        code: 0xa1,
        price: 2 * 375,
        stack_height_required: 3,
        stack_height_change: -3,
        name: "LOG1",
    };
    pub const LOG2: OpCode = OpCode {
        code: 0xa2,
        price: 3 * 375,
        stack_height_required: 4,
        stack_height_change: -4,
        name: "LOG2",
    };
    pub const LOG3: OpCode = OpCode {
        code: 0xa3,
        price: 4 * 375,
        stack_height_required: 5,
        stack_height_change: -5,
        name: "LOG3",
    };
    pub const LOG4: OpCode = OpCode {
        code: 0xa4,
        price: 5 * 375,
        stack_height_required: 6,
        stack_height_change: -6,
        name: "LOG4",
    };
    pub const LT: OpCode = OpCode {
        code: 0x10,
        price: 3,
        stack_height_required: 2,
        stack_height_change: -1,
        name: "LT",
    };
    pub const MLOAD: OpCode = OpCode {
        code: 0x51,
        price: 3,
        stack_height_required: 1,
        stack_height_change: 0,
        name: "MLOAD",
    };
    pub const MOD: OpCode = OpCode {
        code: 0x06,
        price: 5,
        stack_height_required: 2,
        stack_height_change: -1,
        name: "MOD",
    };
    pub const MSIZE: OpCode = OpCode {
        code: 0x59,
        price: 2,
        stack_height_required: 0,
        stack_height_change: 1,
        name: "MSIZE",
    };
    pub const MSTORE: OpCode = OpCode {
        code: 0x52,
        price: 3,
        stack_height_required: 2,
        stack_height_change: -2,
        name: "MSTORE",
    };
    pub const MSTORE8: OpCode = OpCode {
        code: 0x53,
        price: 3,
        stack_height_required: 2,
        stack_height_change: -2,
        name: "MSTORE8",
    };
    pub const MUL: OpCode = OpCode {
        code: 0x02,
        price: 5,
        stack_height_required: 2,
        stack_height_change: -1,
        name: "MUL",
    };
    pub const MULMOD: OpCode = OpCode {
        code: 0x09,
        price: 8,
        stack_height_required: 3,
        stack_height_change: -2,
        name: "MULMOD",
    };
    pub const NOT: OpCode = OpCode {
        code: 0x19,
        price: 3,
        stack_height_required: 1,
        stack_height_change: 0,
        name: "NOT",
    };
    pub const NUMBER: OpCode = OpCode {
        code: 0x43,
        price: 2,
        stack_height_required: 0,
        stack_height_change: 1,
        name: "NUMBER",
    };
    pub const OR: OpCode = OpCode {
        code: 0x17,
        price: 3,
        stack_height_required: 2,
        stack_height_change: -1,
        name: "OR",
    };
    pub const ORIGIN: OpCode = OpCode {
        code: 0x32,
        price: 2,
        stack_height_required: 0,
        stack_height_change: 1,
        name: "ORIGIN",
    };
    pub const PC: OpCode = OpCode {
        code: 0x58,
        price: 2,
        stack_height_required: 0,
        stack_height_change: 1,
        name: "PC",
    };
    pub const POP: OpCode = OpCode {
        code: 0x50,
        price: 2,
        stack_height_required: 1,
        stack_height_change: -1,
        name: "POP",
    };
    pub const PUSH1: OpCode = OpCode {
        code: 0x60,
        price: 3,
        stack_height_required: 0,
        stack_height_change: 1,
        name: "PUSH1",
    };
    pub const PUSH10: OpCode = OpCode {
        code: 0x69,
        price: 3,
        stack_height_required: 0,
        stack_height_change: 1,
        name: "PUSH10",
    };
    pub const PUSH11: OpCode = OpCode {
        code: 0x6a,
        price: 3,
        stack_height_required: 0,
        stack_height_change: 1,
        name: "PUSH11",
    };
    pub const PUSH12: OpCode = OpCode {
        code: 0x6b,
        price: 3,
        stack_height_required: 0,
        stack_height_change: 1,
        name: "PUSH12",
    };
    pub const PUSH13: OpCode = OpCode {
        code: 0x6c,
        price: 3,
        stack_height_required: 0,
        stack_height_change: 1,
        name: "PUSH13",
    };
    pub const PUSH14: OpCode = OpCode {
        code: 0x6d,
        price: 3,
        stack_height_required: 0,
        stack_height_change: 1,
        name: "PUSH14",
    };
    pub const PUSH15: OpCode = OpCode {
        code: 0x6e,
        price: 3,
        stack_height_required: 0,
        stack_height_change: 1,
        name: "PUSH15",
    };
    pub const PUSH16: OpCode = OpCode {
        code: 0x6f,
        price: 3,
        stack_height_required: 0,
        stack_height_change: 1,
        name: "PUSH16",
    };
    pub const PUSH17: OpCode = OpCode {
        code: 0x70,
        price: 3,
        stack_height_required: 0,
        stack_height_change: 1,
        name: "PUSH17",
    };
    pub const PUSH18: OpCode = OpCode {
        code: 0x71,
        price: 3,
        stack_height_required: 0,
        stack_height_change: 1,
        name: "PUSH18",
    };
    pub const PUSH19: OpCode = OpCode {
        code: 0x72,
        price: 3,
        stack_height_required: 0,
        stack_height_change: 1,
        name: "PUSH19",
    };
    pub const PUSH2: OpCode = OpCode {
        code: 0x61,
        price: 3,
        stack_height_required: 0,
        stack_height_change: 1,
        name: "PUSH2",
    };
    pub const PUSH20: OpCode = OpCode {
        code: 0x73,
        price: 3,
        stack_height_required: 0,
        stack_height_change: 1,
        name: "PUSH20",
    };
    pub const PUSH21: OpCode = OpCode {
        code: 0x74,
        price: 3,
        stack_height_required: 0,
        stack_height_change: 1,
        name: "PUSH21",
    };
    pub const PUSH22: OpCode = OpCode {
        code: 0x75,
        price: 3,
        stack_height_required: 0,
        stack_height_change: 1,
        name: "PUSH22",
    };
    pub const PUSH23: OpCode = OpCode {
        code: 0x76,
        price: 3,
        stack_height_required: 0,
        stack_height_change: 1,
        name: "PUSH23",
    };
    pub const PUSH24: OpCode = OpCode {
        code: 0x77,
        price: 3,
        stack_height_required: 0,
        stack_height_change: 1,
        name: "PUSH24",
    };
    pub const PUSH25: OpCode = OpCode {
        code: 0x78,
        price: 3,
        stack_height_required: 0,
        stack_height_change: 1,
        name: "PUSH25",
    };
    pub const PUSH26: OpCode = OpCode {
        code: 0x79,
        price: 3,
        stack_height_required: 0,
        stack_height_change: 1,
        name: "PUSH26",
    };
    pub const PUSH27: OpCode = OpCode {
        code: 0x7a,
        price: 3,
        stack_height_required: 0,
        stack_height_change: 1,
        name: "PUSH27",
    };
    pub const PUSH28: OpCode = OpCode {
        code: 0x7b,
        price: 3,
        stack_height_required: 0,
        stack_height_change: 1,
        name: "PUSH28",
    };
    pub const PUSH29: OpCode = OpCode {
        code: 0x7c,
        price: 3,
        stack_height_required: 0,
        stack_height_change: 1,
        name: "PUSH29",
    };
    pub const PUSH3: OpCode = OpCode {
        code: 0x62,
        price: 3,
        stack_height_required: 0,
        stack_height_change: 1,
        name: "PUSH3",
    };
    pub const PUSH30: OpCode = OpCode {
        code: 0x7d,
        price: 3,
        stack_height_required: 0,
        stack_height_change: 1,
        name: "PUSH30",
    };
    pub const PUSH31: OpCode = OpCode {
        code: 0x7e,
        price: 3,
        stack_height_required: 0,
        stack_height_change: 1,
        name: "PUSH31",
    };
    pub const PUSH32: OpCode = OpCode {
        code: 0x7f,
        price: 3,
        stack_height_required: 0,
        stack_height_change: 1,
        name: "PUSH32",
    };
    pub const PUSH4: OpCode = OpCode {
        code: 0x63,
        price: 3,
        stack_height_required: 0,
        stack_height_change: 1,
        name: "PUSH4",
    };
    pub const PUSH5: OpCode = OpCode {
        code: 0x64,
        price: 3,
        stack_height_required: 0,
        stack_height_change: 1,
        name: "PUSH5",
    };
    pub const PUSH6: OpCode = OpCode {
        code: 0x65,
        price: 3,
        stack_height_required: 0,
        stack_height_change: 1,
        name: "PUSH6",
    };
    pub const PUSH7: OpCode = OpCode {
        code: 0x66,
        price: 3,
        stack_height_required: 0,
        stack_height_change: 1,
        name: "PUSH7",
    };
    pub const PUSH8: OpCode = OpCode {
        code: 0x67,
        price: 3,
        stack_height_required: 0,
        stack_height_change: 1,
        name: "PUSH8",
    };
    pub const PUSH9: OpCode = OpCode {
        code: 0x68,
        price: 3,
        stack_height_required: 0,
        stack_height_change: 1,
        name: "PUSH9",
    };
    pub const RETURN: OpCode = OpCode {
        code: 0xf3,
        price: 0,
        stack_height_required: 2,
        stack_height_change: -2,
        name: "RETURN",
    };
    pub const RETURNDATACOPY: OpCode = OpCode {
        code: 0x3e,
        price: 3,
        stack_height_required: 3,
        stack_height_change: -3,
        name: "RETURNDATACOPY",
    };
    pub const RETURNDATASIZE: OpCode = OpCode {
        code: 0x3d,
        price: 2,
        stack_height_required: 0,
        stack_height_change: 1,
        name: "RETURNDATASIZE",
    };
    pub const REVERT: OpCode = OpCode {
        code: 0xfd,
        price: 0,
        stack_height_required: 2,
        stack_height_change: -2,
        name: "REVERT",
    };
    pub const SAR: OpCode = OpCode {
        code: 0x1d,
        price: 3,
        stack_height_required: 2,
        stack_height_change: -1,
        name: "SAR",
    };
    pub const SDIV: OpCode = OpCode {
        code: 0x05,
        price: 5,
        stack_height_required: 2,
        stack_height_change: -1,
        name: "SDIV",
    };
    pub const SELFBALANCE: OpCode = OpCode {
        code: 0x47,
        price: 5,
        stack_height_required: 0,
        stack_height_change: 1,
        name: "SELFBALANCE",
    };
    pub const SELFDESTRUCT: OpCode = OpCode {
        code: 0xff,
        price: 5000,
        stack_height_required: 1,
        stack_height_change: -1,
        name: "SELFDESTRUCT",
    };
    pub const SGT: OpCode = OpCode {
        code: 0x13,
        price: 3,
        stack_height_required: 2,
        stack_height_change: -1,
        name: "SGT",
    };
    pub const SHL: OpCode = OpCode {
        code: 0x1b,
        price: 3,
        stack_height_required: 2,
        stack_height_change: -1,
        name: "SHL",
    };
    pub const SHR: OpCode = OpCode {
        code: 0x1c,
        price: 3,
        stack_height_required: 2,
        stack_height_change: -1,
        name: "SHR",
    };
    pub const SIGNEXTEND: OpCode = OpCode {
        code: 0x0b,
        price: 5,
        stack_height_required: 2,
        stack_height_change: -1,
        name: "SIGNEXTEND",
    };
    pub const SLOAD: OpCode = OpCode {
        code: 0x54,
        price: WARM_STORAGE_READ_COST,
        stack_height_required: 1,
        stack_height_change: 0,
        name: "SLOAD",
    };
    pub const SLT: OpCode = OpCode {
        code: 0x12,
        price: 3,
        stack_height_required: 2,
        stack_height_change: -1,
        name: "SLT",
    };
    pub const SMOD: OpCode = OpCode {
        code: 0x07,
        price: 5,
        stack_height_required: 2,
        stack_height_change: -1,
        name: "SMOD",
    };
    pub const SSTORE: OpCode = OpCode {
        code: 0x55,
        price: 0,
        stack_height_required: 2,
        stack_height_change: -2,
        name: "SSTORE",
    };
    pub const STATICCALL: OpCode = OpCode {
        code: 0xfa,
        price: WARM_STORAGE_READ_COST,
        stack_height_required: 6,
        stack_height_change: -5,
        name: "STATICCALL",
    };
    pub const STOP: OpCode = OpCode {
        code: 0x00,
        price: 0,
        stack_height_required: 0,
        stack_height_change: 0,
        name: "STOP",
    };
    pub const SUB: OpCode = OpCode {
        code: 0x03,
        price: 3,
        stack_height_required: 2,
        stack_height_change: -1,
        name: "SUB",
    };
    pub const SWAP1: OpCode = OpCode {
        code: 0x90,
        price: 3,
        stack_height_required: 2,
        stack_height_change: 0,
        name: "SWAP1",
    };
    pub const SWAP10: OpCode = OpCode {
        code: 0x99,
        price: 3,
        stack_height_required: 11,
        stack_height_change: 0,
        name: "SWAP10",
    };
    pub const SWAP11: OpCode = OpCode {
        code: 0x9a,
        price: 3,
        stack_height_required: 12,
        stack_height_change: 0,
        name: "SWAP11",
    };
    pub const SWAP12: OpCode = OpCode {
        code: 0x9b,
        price: 3,
        stack_height_required: 13,
        stack_height_change: 0,
        name: "SWAP12",
    };
    pub const SWAP13: OpCode = OpCode {
        code: 0x9c,
        price: 3,
        stack_height_required: 14,
        stack_height_change: 0,
        name: "SWAP13",
    };
    pub const SWAP14: OpCode = OpCode {
        code: 0x9d,
        price: 3,
        stack_height_required: 15,
        stack_height_change: 0,
        name: "SWAP14",
    };
    pub const SWAP15: OpCode = OpCode {
        code: 0x9e,
        price: 3,
        stack_height_required: 16,
        stack_height_change: 0,
        name: "SWAP15",
    };
    pub const SWAP16: OpCode = OpCode {
        code: 0x9f,
        price: 3,
        stack_height_required: 17,
        stack_height_change: 0,
        name: "SWAP16",
    };
    pub const SWAP2: OpCode = OpCode {
        code: 0x91,
        price: 3,
        stack_height_required: 3,
        stack_height_change: 0,
        name: "SWAP2",
    };
    pub const SWAP3: OpCode = OpCode {
        code: 0x92,
        price: 3,
        stack_height_required: 4,
        stack_height_change: 0,
        name: "SWAP3",
    };
    pub const SWAP4: OpCode = OpCode {
        code: 0x93,
        price: 3,
        stack_height_required: 5,
        stack_height_change: 0,
        name: "SWAP4",
    };
    pub const SWAP5: OpCode = OpCode {
        code: 0x94,
        price: 3,
        stack_height_required: 6,
        stack_height_change: 0,
        name: "SWAP5",
    };
    pub const SWAP6: OpCode = OpCode {
        code: 0x95,
        price: 3,
        stack_height_required: 7,
        stack_height_change: 0,
        name: "SWAP6",
    };
    pub const SWAP7: OpCode = OpCode {
        code: 0x96,
        price: 3,
        stack_height_required: 8,
        stack_height_change: 0,
        name: "SWAP7",
    };
    pub const SWAP8: OpCode = OpCode {
        code: 0x97,
        price: 3,
        stack_height_required: 9,
        stack_height_change: 0,
        name: "SWAP8",
    };
    pub const SWAP9: OpCode = OpCode {
        code: 0x98,
        price: 3,
        stack_height_required: 10,
        stack_height_change: 0,
        name: "SWAP9",
    };
    pub const TIMESTAMP: OpCode = OpCode {
        code: 0x42,
        price: 2,
        stack_height_required: 0,
        stack_height_change: 1,
        name: "TIMESTAMP",
    };
    pub const XOR: OpCode = OpCode {
        code: 0x18,
        price: 3,
        stack_height_required: 2,
        stack_height_change: -1,
        name: "XOR",
    };
}

impl std::fmt::Display for OpCode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.name)
    }
}

impl TryFrom<u8> for OpCode {
    type Error = StatusCode;

    fn try_from(value: u8) -> Result<Self, Self::Error> {
        // todo: optimize and turn it into a jump table
        const OPCODES: [OpCode; 143] = [
            OpCode::STOP,
            OpCode::ADD,
            OpCode::MUL,
            OpCode::SUB,
            OpCode::DIV,
            OpCode::SDIV,
            OpCode::MOD,
            OpCode::SMOD,
            OpCode::ADDMOD,
            OpCode::MULMOD,
            OpCode::EXP,
            OpCode::SIGNEXTEND,
            OpCode::LT,
            OpCode::GT,
            OpCode::SLT,
            OpCode::SGT,
            OpCode::EQ,
            OpCode::ISZERO,
            OpCode::AND,
            OpCode::OR,
            OpCode::XOR,
            OpCode::NOT,
            OpCode::BYTE,
            OpCode::SHL,
            OpCode::SHR,
            OpCode::SAR,
            OpCode::KECCAK256,
            OpCode::ADDRESS,
            OpCode::BALANCE,
            OpCode::ORIGIN,
            OpCode::CALLER,
            OpCode::CALLVALUE,
            OpCode::CALLDATALOAD,
            OpCode::CALLDATASIZE,
            OpCode::CALLDATACOPY,
            OpCode::CODESIZE,
            OpCode::CODECOPY,
            OpCode::GASPRICE,
            OpCode::EXTCODESIZE,
            OpCode::EXTCODECOPY,
            OpCode::RETURNDATASIZE,
            OpCode::RETURNDATACOPY,
            OpCode::EXTCODEHASH,
            OpCode::BLOCKHASH,
            OpCode::COINBASE,
            OpCode::TIMESTAMP,
            OpCode::NUMBER,
            OpCode::DIFFICULTY,
            OpCode::GASLIMIT,
            OpCode::CHAINID,
            OpCode::SELFBALANCE,
            OpCode::BASEFEE,
            OpCode::POP,
            OpCode::MLOAD,
            OpCode::MSTORE,
            OpCode::MSTORE8,
            OpCode::SLOAD,
            OpCode::SSTORE,
            OpCode::JUMP,
            OpCode::JUMPI,
            OpCode::PC,
            OpCode::MSIZE,
            OpCode::GAS,
            OpCode::JUMPDEST,
            OpCode::PUSH1,
            OpCode::PUSH2,
            OpCode::PUSH3,
            OpCode::PUSH4,
            OpCode::PUSH5,
            OpCode::PUSH6,
            OpCode::PUSH7,
            OpCode::PUSH8,
            OpCode::PUSH9,
            OpCode::PUSH10,
            OpCode::PUSH11,
            OpCode::PUSH12,
            OpCode::PUSH13,
            OpCode::PUSH14,
            OpCode::PUSH15,
            OpCode::PUSH16,
            OpCode::PUSH17,
            OpCode::PUSH18,
            OpCode::PUSH19,
            OpCode::PUSH20,
            OpCode::PUSH21,
            OpCode::PUSH22,
            OpCode::PUSH23,
            OpCode::PUSH24,
            OpCode::PUSH25,
            OpCode::PUSH26,
            OpCode::PUSH27,
            OpCode::PUSH28,
            OpCode::PUSH29,
            OpCode::PUSH30,
            OpCode::PUSH31,
            OpCode::PUSH32,
            OpCode::DUP1,
            OpCode::DUP2,
            OpCode::DUP3,
            OpCode::DUP4,
            OpCode::DUP5,
            OpCode::DUP6,
            OpCode::DUP7,
            OpCode::DUP8,
            OpCode::DUP9,
            OpCode::DUP10,
            OpCode::DUP11,
            OpCode::DUP12,
            OpCode::DUP13,
            OpCode::DUP14,
            OpCode::DUP15,
            OpCode::DUP16,
            OpCode::SWAP1,
            OpCode::SWAP2,
            OpCode::SWAP3,
            OpCode::SWAP4,
            OpCode::SWAP5,
            OpCode::SWAP6,
            OpCode::SWAP7,
            OpCode::SWAP8,
            OpCode::SWAP9,
            OpCode::SWAP10,
            OpCode::SWAP11,
            OpCode::SWAP12,
            OpCode::SWAP13,
            OpCode::SWAP14,
            OpCode::SWAP15,
            OpCode::SWAP16,
            OpCode::LOG0,
            OpCode::LOG1,
            OpCode::LOG2,
            OpCode::LOG3,
            OpCode::LOG4,
            OpCode::CREATE,
            OpCode::CALL,
            OpCode::CALLCODE,
            OpCode::RETURN,
            OpCode::DELEGATECALL,
            OpCode::CREATE2,
            OpCode::STATICCALL,
            OpCode::REVERT,
            OpCode::INVALID,
            OpCode::SELFDESTRUCT,
        ];

        for op in OPCODES {
            if op == value {
                return Ok(op);
            }
        }

        Err(StatusCode::UndefinedInstruction)
    }
}
