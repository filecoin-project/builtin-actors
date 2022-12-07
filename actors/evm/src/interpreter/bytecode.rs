#![allow(dead_code)]

use std::ops::Deref;

use super::execution::opcodes;

#[derive(Clone, Debug)]
pub struct Bytecode {
    code: Vec<u8>,
    jumpdest: Vec<bool>,
}

impl Bytecode {
    pub fn new(bytecode: Vec<u8>) -> Self {
        // only jumps to those addresses are valid. This is a security
        // feature by EVM to disallow jumps to arbitary code addresses.
        let mut jumpdest = vec![false; bytecode.len()];
        let mut i = 0;
        while i < bytecode.len() {
            if bytecode[i] == opcodes::JUMPDEST as u8 {
                jumpdest[i] = true;
                i += 1;
            } else if bytecode[i] >= opcodes::PUSH1 && bytecode[i] <= opcodes::PUSH32 {
                i += (bytecode[i] - opcodes::PUSH1) as usize + 2;
            } else {
                i += 1;
            }
        }

        Self { code: bytecode, jumpdest }
    }

    /// Checks if the EVM is allowed to jump to this location.
    ///
    /// This location must begin with a JUMPDEST opcode that
    /// marks a valid jump destination
    pub fn valid_jump_destination(&self, offset: usize) -> bool {
        offset < self.jumpdest.len() && self.jumpdest[offset]
    }
}

impl Deref for Bytecode {
    type Target = [u8];

    fn deref(&self) -> &Self::Target {
        &self.code
    }
}

impl AsRef<[u8]> for Bytecode {
    fn as_ref(&self) -> &[u8] {
        &self.code
    }
}
