//! EVM Opcodes as of Berlin Hard Fork
//!
//! On filecoin we will never have to replay blocks that are older
//! than the release date of the FVM-EVM runtime, so supporting
//! historic behavior is not needed.

use crate::interpreter::output::StatusCode;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct OpCode {
    /// the byte representing the opcode in binary
    pub code: u8,

    /// The number of stack items the instruction accesses during execution.
    pub stack_height_required: u8,

    /// The stack height change caused by the instruction execution. Can be
    /// negative.
    pub stack_height_change: i8,

    /// Human readable name of the opcode.
    pub name: &'static str,

    /// Reserved/Undefined opcode indicator
    pub reserved: bool,
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

macro_rules! def_opcode {
    ($id:ident, $code:literal, $sk_required:literal, $sk_change:literal) => {
        pub const $id: OpCode = OpCode {
            code: $code,
            stack_height_required: $sk_required,
            stack_height_change: $sk_change,
            name: stringify!($id),
            reserved: false,
        };
    };
}

macro_rules! def_reserved {
    ($id:ident, $code:literal) => {
        pub const $id: OpCode = OpCode {
            code: $code,
            stack_height_required: 0,
            stack_height_change: 0,
            name: "RESERVED",
            reserved: true,
        };
    };
}

impl OpCode {
    def_opcode!(STOP, 0x00, 0, 0);
    def_opcode!(ADD, 0x01, 2, -1);
    def_opcode!(MUL, 0x02, 2, -1);
    def_opcode!(SUB, 0x03, 2, -1);
    def_opcode!(DIV, 0x04, 2, -1);
    def_opcode!(SDIV, 0x05, 2, -1);
    def_opcode!(MOD, 0x06, 2, -1);
    def_opcode!(SMOD, 0x07, 2, -1);
    def_opcode!(ADDMOD, 0x08, 3, -2);
    def_opcode!(MULMOD, 0x09, 3, -2);
    def_opcode!(EXP, 0x0a, 2, -1);
    def_opcode!(SIGNEXTEND, 0x0b, 2, -1);
    def_reserved!(RESERVED_0C, 0x0c);
    def_reserved!(RESERVED_0D, 0x0d);
    def_reserved!(RESERVED_0E, 0x0e);
    def_reserved!(RESERVED_0F, 0x0f);
    def_opcode!(LT, 0x10, 2, -1);
    def_opcode!(GT, 0x11, 2, -1);
    def_opcode!(SLT, 0x12, 2, -1);
    def_opcode!(SGT, 0x13, 2, -1);
    def_opcode!(EQ, 0x14, 2, -1);
    def_opcode!(ISZERO, 0x15, 1, 0);
    def_opcode!(AND, 0x16, 2, -1);
    def_opcode!(OR, 0x17, 2, -1);
    def_opcode!(XOR, 0x18, 2, -1);
    def_opcode!(NOT, 0x19, 1, 0);
    def_opcode!(BYTE, 0x1a, 2, -1);
    def_opcode!(SHL, 0x1b, 2, -1);
    def_opcode!(SHR, 0x1c, 2, -1);
    def_opcode!(SAR, 0x1d, 2, -1);
    def_reserved!(RESERVED_1E, 0x1e);
    def_reserved!(RESERVED_1F, 0x1f);
    def_opcode!(KECCAK256, 0x20, 2, -1); // SHA3
    def_reserved!(RESERVED_21, 0x21);
    def_reserved!(RESERVED_22, 0x22);
    def_reserved!(RESERVED_23, 0x23);
    def_reserved!(RESERVED_24, 0x24);
    def_reserved!(RESERVED_25, 0x25);
    def_reserved!(RESERVED_26, 0x26);
    def_reserved!(RESERVED_27, 0x27);
    def_reserved!(RESERVED_28, 0x28);
    def_reserved!(RESERVED_29, 0x29);
    def_reserved!(RESERVED_2A, 0x2a);
    def_reserved!(RESERVED_2B, 0x2b);
    def_reserved!(RESERVED_2C, 0x2c);
    def_reserved!(RESERVED_2D, 0x2d);
    def_reserved!(RESERVED_2E, 0x2e);
    def_reserved!(RESERVED_2F, 0x2f);
    def_opcode!(ADDRESS, 0x30, 0, 1);
    def_opcode!(BALANCE, 0x31, 1, 0);
    def_opcode!(ORIGIN, 0x32, 0, 1);
    def_opcode!(CALLER, 0x33, 0, 1);
    def_opcode!(CALLVALUE, 0x34, 0, 1);
    def_opcode!(CALLDATALOAD, 0x35, 1, 0);
    def_opcode!(CALLDATASIZE, 0x36, 0, 1);
    def_opcode!(CALLDATACOPY, 0x37, 3, -3);
    def_opcode!(CODESIZE, 0x38, 0, 1);
    def_opcode!(CODECOPY, 0x39, 3, -3);
    def_opcode!(GASPRICE, 0x3a, 0, 1);
    def_opcode!(EXTCODESIZE, 0x3b, 1, 0);
    def_opcode!(EXTCODECOPY, 0x3c, 4, -4);
    def_opcode!(RETURNDATASIZE, 0x3d, 0, 1);
    def_opcode!(RETURNDATACOPY, 0x3e, 3, -3);
    def_opcode!(EXTCODEHASH, 0x3f, 1, 0);
    def_opcode!(BLOCKHASH, 0x40, 1, 0);
    def_opcode!(COINBASE, 0x41, 0, 1);
    def_opcode!(TIMESTAMP, 0x42, 0, 1);
    def_opcode!(NUMBER, 0x43, 0, 1);
    def_opcode!(DIFFICULTY, 0x44, 0, 1);
    def_opcode!(GASLIMIT, 0x45, 0, 1);
    def_opcode!(CHAINID, 0x46, 0, 1);
    def_opcode!(SELFBALANCE, 0x47, 0, 1);
    def_opcode!(BASEFEE, 0x48, 0, 1);
    def_reserved!(RESERVED_49, 0x49);
    def_reserved!(RESERVED_4A, 0x4a);
    def_reserved!(RESERVED_4B, 0x4b);
    def_reserved!(RESERVED_4C, 0x4c);
    def_reserved!(RESERVED_4D, 0x4d);
    def_reserved!(RESERVED_4E, 0x4e);
    def_reserved!(RESERVED_4F, 0x4f);
    def_opcode!(POP, 0x50, 1, -1);
    def_opcode!(MLOAD, 0x51, 1, 0);
    def_opcode!(MSTORE, 0x52, 2, -2);
    def_opcode!(MSTORE8, 0x53, 2, -2);
    def_opcode!(SLOAD, 0x54, 1, 0);
    def_opcode!(SSTORE, 0x55, 2, -2);
    def_opcode!(JUMP, 0x56, 1, -1);
    def_opcode!(JUMPI, 0x57, 2, -2);
    def_opcode!(PC, 0x58, 0, 1);
    def_opcode!(MSIZE, 0x59, 0, 1);
    def_opcode!(GAS, 0x5a, 0, 1);
    def_opcode!(JUMPDEST, 0x5b, 0, 0);
    def_reserved!(RESERVED_5C, 0x5c);
    def_reserved!(RESERVED_5D, 0x5d);
    def_reserved!(RESERVED_5E, 0x5e);
    def_reserved!(RESERVED_5F, 0x5f);
    def_opcode!(PUSH1, 0x60, 0, 1);
    def_opcode!(PUSH2, 0x61, 0, 1);
    def_opcode!(PUSH3, 0x62, 0, 1);
    def_opcode!(PUSH4, 0x63, 0, 1);
    def_opcode!(PUSH5, 0x64, 0, 1);
    def_opcode!(PUSH6, 0x65, 0, 1);
    def_opcode!(PUSH7, 0x66, 0, 1);
    def_opcode!(PUSH8, 0x67, 0, 1);
    def_opcode!(PUSH9, 0x68, 0, 1);
    def_opcode!(PUSH10, 0x69, 0, 1);
    def_opcode!(PUSH11, 0x6a, 0, 1);
    def_opcode!(PUSH12, 0x6b, 0, 1);
    def_opcode!(PUSH13, 0x6c, 0, 1);
    def_opcode!(PUSH14, 0x6d, 0, 1);
    def_opcode!(PUSH15, 0x6e, 0, 1);
    def_opcode!(PUSH16, 0x6f, 0, 1);
    def_opcode!(PUSH17, 0x70, 0, 1);
    def_opcode!(PUSH18, 0x71, 0, 1);
    def_opcode!(PUSH19, 0x72, 0, 1);
    def_opcode!(PUSH20, 0x73, 0, 1);
    def_opcode!(PUSH21, 0x74, 0, 1);
    def_opcode!(PUSH22, 0x75, 0, 1);
    def_opcode!(PUSH23, 0x76, 0, 1);
    def_opcode!(PUSH24, 0x77, 0, 1);
    def_opcode!(PUSH25, 0x78, 0, 1);
    def_opcode!(PUSH26, 0x79, 0, 1);
    def_opcode!(PUSH27, 0x7a, 0, 1);
    def_opcode!(PUSH28, 0x7b, 0, 1);
    def_opcode!(PUSH29, 0x7c, 0, 1);
    def_opcode!(PUSH30, 0x7d, 0, 1);
    def_opcode!(PUSH31, 0x7e, 0, 1);
    def_opcode!(PUSH32, 0x7f, 0, 1);
    def_opcode!(DUP1, 0x80, 1, 1);
    def_opcode!(DUP2, 0x81, 2, 1);
    def_opcode!(DUP3, 0x82, 3, 1);
    def_opcode!(DUP4, 0x83, 4, 1);
    def_opcode!(DUP5, 0x84, 5, 1);
    def_opcode!(DUP6, 0x85, 6, 1);
    def_opcode!(DUP7, 0x86, 7, 1);
    def_opcode!(DUP8, 0x87, 8, 1);
    def_opcode!(DUP9, 0x88, 9, 1);
    def_opcode!(DUP10, 0x89, 10, 1);
    def_opcode!(DUP11, 0x8a, 11, 1);
    def_opcode!(DUP12, 0x8b, 12, 1);
    def_opcode!(DUP13, 0x8c, 13, 1);
    def_opcode!(DUP14, 0x8d, 14, 1);
    def_opcode!(DUP15, 0x8e, 15, 1);
    def_opcode!(DUP16, 0x8f, 16, 1);
    def_opcode!(SWAP1, 0x90, 2, 0);
    def_opcode!(SWAP2, 0x91, 3, 0);
    def_opcode!(SWAP3, 0x92, 4, 0);
    def_opcode!(SWAP4, 0x93, 5, 0);
    def_opcode!(SWAP5, 0x94, 6, 0);
    def_opcode!(SWAP6, 0x95, 7, 0);
    def_opcode!(SWAP7, 0x96, 8, 0);
    def_opcode!(SWAP8, 0x97, 9, 0);
    def_opcode!(SWAP9, 0x98, 10, 0);
    def_opcode!(SWAP10, 0x99, 11, 0);
    def_opcode!(SWAP11, 0x9a, 12, 0);
    def_opcode!(SWAP12, 0x9b, 13, 0);
    def_opcode!(SWAP13, 0x9c, 14, 0);
    def_opcode!(SWAP14, 0x9d, 15, 0);
    def_opcode!(SWAP15, 0x9e, 16, 0);
    def_opcode!(SWAP16, 0x9f, 17, 0);
    def_opcode!(LOG0, 0xa0, 2, -2);
    def_opcode!(LOG1, 0xa1, 3, -3);
    def_opcode!(LOG2, 0xa2, 4, -4);
    def_opcode!(LOG3, 0xa3, 5, -5);
    def_opcode!(LOG4, 0xa4, 6, -6);
    def_reserved!(RESERVED_A5, 0xa5);
    def_reserved!(RESERVED_A6, 0xa6);
    def_reserved!(RESERVED_A7, 0xa7);
    def_reserved!(RESERVED_A8, 0xa8);
    def_reserved!(RESERVED_A9, 0xa9);
    def_reserved!(RESERVED_AA, 0xaa);
    def_reserved!(RESERVED_AB, 0xab);
    def_reserved!(RESERVED_AC, 0xac);
    def_reserved!(RESERVED_AD, 0xad);
    def_reserved!(RESERVED_AE, 0xae);
    def_reserved!(RESERVED_AF, 0xaf);
    def_reserved!(RESERVED_B0, 0xb0);
    def_reserved!(RESERVED_B1, 0xb1);
    def_reserved!(RESERVED_B2, 0xb2);
    def_reserved!(RESERVED_B3, 0xb3);
    def_reserved!(RESERVED_B4, 0xb4);
    def_reserved!(RESERVED_B5, 0xb5);
    def_reserved!(RESERVED_B6, 0xb6);
    def_reserved!(RESERVED_B7, 0xb7);
    def_reserved!(RESERVED_B8, 0xb8);
    def_reserved!(RESERVED_B9, 0xb9);
    def_reserved!(RESERVED_BA, 0xba);
    def_reserved!(RESERVED_BB, 0xbb);
    def_reserved!(RESERVED_BC, 0xbc);
    def_reserved!(RESERVED_BD, 0xbd);
    def_reserved!(RESERVED_BE, 0xbe);
    def_reserved!(RESERVED_BF, 0xbf);
    def_reserved!(RESERVED_C0, 0xc0);
    def_reserved!(RESERVED_C1, 0xc1);
    def_reserved!(RESERVED_C2, 0xc2);
    def_reserved!(RESERVED_C3, 0xc3);
    def_reserved!(RESERVED_C4, 0xc4);
    def_reserved!(RESERVED_C5, 0xc5);
    def_reserved!(RESERVED_C6, 0xc6);
    def_reserved!(RESERVED_C7, 0xc7);
    def_reserved!(RESERVED_C8, 0xc8);
    def_reserved!(RESERVED_C9, 0xc9);
    def_reserved!(RESERVED_CA, 0xca);
    def_reserved!(RESERVED_CB, 0xcb);
    def_reserved!(RESERVED_CC, 0xcc);
    def_reserved!(RESERVED_CD, 0xcd);
    def_reserved!(RESERVED_CE, 0xce);
    def_reserved!(RESERVED_CF, 0xcf);
    def_reserved!(RESERVED_D0, 0xd0);
    def_reserved!(RESERVED_D1, 0xd1);
    def_reserved!(RESERVED_D2, 0xd2);
    def_reserved!(RESERVED_D3, 0xd3);
    def_reserved!(RESERVED_D4, 0xd4);
    def_reserved!(RESERVED_D5, 0xd5);
    def_reserved!(RESERVED_D6, 0xd6);
    def_reserved!(RESERVED_D7, 0xd7);
    def_reserved!(RESERVED_D8, 0xd8);
    def_reserved!(RESERVED_D9, 0xd9);
    def_reserved!(RESERVED_DA, 0xda);
    def_reserved!(RESERVED_DB, 0xdb);
    def_reserved!(RESERVED_DC, 0xdc);
    def_reserved!(RESERVED_DD, 0xdd);
    def_reserved!(RESERVED_DE, 0xde);
    def_reserved!(RESERVED_DF, 0xdf);
    def_reserved!(RESERVED_E0, 0xe0);
    def_reserved!(RESERVED_E1, 0xe1);
    def_reserved!(RESERVED_E2, 0xe2);
    def_reserved!(RESERVED_E3, 0xe3);
    def_reserved!(RESERVED_E4, 0xe4);
    def_reserved!(RESERVED_E5, 0xe5);
    def_reserved!(RESERVED_E6, 0xe6);
    def_reserved!(RESERVED_E7, 0xe7);
    def_reserved!(RESERVED_E8, 0xe8);
    def_reserved!(RESERVED_E9, 0xe9);
    def_reserved!(RESERVED_EA, 0xea);
    def_reserved!(RESERVED_EB, 0xeb);
    def_reserved!(RESERVED_EC, 0xec);
    def_reserved!(RESERVED_ED, 0xed);
    def_reserved!(RESERVED_EE, 0xee);
    def_reserved!(RESERVED_EF, 0xef);
    def_opcode!(CREATE, 0xf0, 3, -2);
    def_opcode!(CALL, 0xf1, 7, -6);
    def_opcode!(CALLCODE, 0xf2, 7, -6);
    def_opcode!(RETURN, 0xf3, 2, -2);
    def_opcode!(DELEGATECALL, 0xf4, 6, -5);
    def_opcode!(CREATE2, 0xf5, 4, -3);
    def_reserved!(RESERVED_F6, 0xf6);
    def_reserved!(RESERVED_F7, 0xf7);
    def_reserved!(RESERVED_F8, 0xf8);
    def_reserved!(RESERVED_F9, 0xf9);
    def_opcode!(STATICCALL, 0xfa, 6, -5);
    def_reserved!(RESERVED_FB, 0xfb);
    def_reserved!(RESERVED_FC, 0xfc);
    def_opcode!(REVERT, 0xfd, 2, -2);
    def_opcode!(INVALID, 0xfe, 0, 0);
    def_opcode!(SELFDESTRUCT, 0xff, 1, -1);

    const OPCODES: [OpCode; 256] = [
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
        OpCode::RESERVED_0C,
        OpCode::RESERVED_0D,
        OpCode::RESERVED_0E,
        OpCode::RESERVED_0F,
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
        OpCode::RESERVED_1E,
        OpCode::RESERVED_1F,
        OpCode::KECCAK256,
        OpCode::RESERVED_21,
        OpCode::RESERVED_22,
        OpCode::RESERVED_23,
        OpCode::RESERVED_24,
        OpCode::RESERVED_25,
        OpCode::RESERVED_26,
        OpCode::RESERVED_27,
        OpCode::RESERVED_28,
        OpCode::RESERVED_29,
        OpCode::RESERVED_2A,
        OpCode::RESERVED_2B,
        OpCode::RESERVED_2C,
        OpCode::RESERVED_2D,
        OpCode::RESERVED_2E,
        OpCode::RESERVED_2F,
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
        OpCode::RESERVED_49,
        OpCode::RESERVED_4A,
        OpCode::RESERVED_4B,
        OpCode::RESERVED_4C,
        OpCode::RESERVED_4D,
        OpCode::RESERVED_4E,
        OpCode::RESERVED_4F,
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
        OpCode::RESERVED_5C,
        OpCode::RESERVED_5D,
        OpCode::RESERVED_5E,
        OpCode::RESERVED_5F,
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
        OpCode::RESERVED_A5,
        OpCode::RESERVED_A6,
        OpCode::RESERVED_A7,
        OpCode::RESERVED_A8,
        OpCode::RESERVED_A9,
        OpCode::RESERVED_AA,
        OpCode::RESERVED_AB,
        OpCode::RESERVED_AC,
        OpCode::RESERVED_AD,
        OpCode::RESERVED_AE,
        OpCode::RESERVED_AF,
        OpCode::RESERVED_B0,
        OpCode::RESERVED_B1,
        OpCode::RESERVED_B2,
        OpCode::RESERVED_B3,
        OpCode::RESERVED_B4,
        OpCode::RESERVED_B5,
        OpCode::RESERVED_B6,
        OpCode::RESERVED_B7,
        OpCode::RESERVED_B8,
        OpCode::RESERVED_B9,
        OpCode::RESERVED_BA,
        OpCode::RESERVED_BB,
        OpCode::RESERVED_BC,
        OpCode::RESERVED_BD,
        OpCode::RESERVED_BE,
        OpCode::RESERVED_BF,
        OpCode::RESERVED_C0,
        OpCode::RESERVED_C1,
        OpCode::RESERVED_C2,
        OpCode::RESERVED_C3,
        OpCode::RESERVED_C4,
        OpCode::RESERVED_C5,
        OpCode::RESERVED_C6,
        OpCode::RESERVED_C7,
        OpCode::RESERVED_C8,
        OpCode::RESERVED_C9,
        OpCode::RESERVED_CA,
        OpCode::RESERVED_CB,
        OpCode::RESERVED_CC,
        OpCode::RESERVED_CD,
        OpCode::RESERVED_CE,
        OpCode::RESERVED_CF,
        OpCode::RESERVED_D0,
        OpCode::RESERVED_D1,
        OpCode::RESERVED_D2,
        OpCode::RESERVED_D3,
        OpCode::RESERVED_D4,
        OpCode::RESERVED_D5,
        OpCode::RESERVED_D6,
        OpCode::RESERVED_D7,
        OpCode::RESERVED_D8,
        OpCode::RESERVED_D9,
        OpCode::RESERVED_DA,
        OpCode::RESERVED_DB,
        OpCode::RESERVED_DC,
        OpCode::RESERVED_DD,
        OpCode::RESERVED_DE,
        OpCode::RESERVED_DF,
        OpCode::RESERVED_E0,
        OpCode::RESERVED_E1,
        OpCode::RESERVED_E2,
        OpCode::RESERVED_E3,
        OpCode::RESERVED_E4,
        OpCode::RESERVED_E5,
        OpCode::RESERVED_E6,
        OpCode::RESERVED_E7,
        OpCode::RESERVED_E8,
        OpCode::RESERVED_E9,
        OpCode::RESERVED_EA,
        OpCode::RESERVED_EB,
        OpCode::RESERVED_EC,
        OpCode::RESERVED_ED,
        OpCode::RESERVED_EE,
        OpCode::RESERVED_EF,
        OpCode::CREATE,
        OpCode::CALL,
        OpCode::CALLCODE,
        OpCode::RETURN,
        OpCode::DELEGATECALL,
        OpCode::CREATE2,
        OpCode::RESERVED_F6,
        OpCode::RESERVED_F7,
        OpCode::RESERVED_F8,
        OpCode::RESERVED_F9,
        OpCode::STATICCALL,
        OpCode::RESERVED_FB,
        OpCode::RESERVED_FC,
        OpCode::REVERT,
        OpCode::INVALID,
        OpCode::SELFDESTRUCT,
    ];
}

impl std::fmt::Display for OpCode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.name)
    }
}

impl TryFrom<u8> for OpCode {
    type Error = StatusCode;

    fn try_from(op: u8) -> Result<Self, Self::Error> {
        let opc = OpCode::OPCODES[op as usize];

        if opc.reserved {
            return Err(StatusCode::UndefinedInstruction);
        }

        Ok(opc)
    }
}
