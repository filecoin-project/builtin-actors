#![allow(dead_code)]

use {super::opcode::OpCode, std::ops::Deref};

pub struct Bytecode<'c> {
    code: &'c [u8],
}

impl<'c> Bytecode<'c> {
    pub fn new(bytecode: &'c [u8]) -> Self {
        Self { code: bytecode }
    }

    /// Checks if the EVM is allowed to jump to this location.
    ///
    /// This location must begin with a JUMPDEST opcode that
    /// marks a valid jump destination
    pub fn valid_jump_destination(&self, offset: usize) -> bool {
        offset < self.code.len() && self.code[offset] == OpCode::JUMPDEST as u8
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
