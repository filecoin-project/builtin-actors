#![allow(dead_code)]

use {super::opcode::OpCode, crate::interp::output::StatusCode, std::ops::Deref};

pub struct Bytecode<'c> {
    code: &'c [u8],
    jumpdest: Vec<bool>,
}

impl<'c> Bytecode<'c> {
    pub fn new(bytecode: &'c [u8]) -> Result<Self, StatusCode> {
        // only jumps to those addresses are valid. This is a security
        // feature by EVM to disallow jumps to arbitary code addresses.
        // todo: create the jumpdest table only once during initial contract deployment
        let mut jumpdest = vec![false; bytecode.len()];
        let mut i = 0;
        while i < bytecode.len() {
            if bytecode[i] == OpCode::JUMPDEST.code {
                jumpdest[i] = true;
                i += 1;
            } else if bytecode[i] >= OpCode::PUSH1.code && bytecode[i] <= OpCode::PUSH32.code {
                i += (bytecode[i] - OpCode::PUSH1.code) as usize + 2;
            } else {
                i += 1;
            }
        }

        Ok(Self { code: bytecode, jumpdest })
    }

    /// Checks if the EVM is allowed to jump to this location.
    ///
    /// This location must begin with a JUMPDEST opcode that
    /// marks a valid jump destination
    pub fn valid_jump_destination(&self, offset: usize) -> bool {
        offset < self.jumpdest.len() && self.jumpdest[offset]
    }
}

impl<'c> Deref for Bytecode<'c> {
    type Target = [u8];

    fn deref(&self) -> &'c Self::Target {
        self.code
    }
}

impl<'c> AsRef<[u8]> for Bytecode<'c> {
    fn as_ref(&self) -> &'c [u8] {
        self.code
    }
}
