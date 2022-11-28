#![allow(clippy::missing_safety_doc)]
use {crate::interpreter::stack::Stack, crate::interpreter::U256};

#[inline]
pub(crate) unsafe fn push<const LEN: usize>(stack: &mut Stack, code: &[u8]) -> usize {
    let pushval = &code[..LEN];
    stack.push(match pushval.len() {
        0 => U256::zero(),
        32 => U256::from_big_endian(pushval),
        _ => {
            let mut padded = [0; 32];
            padded[32 - pushval.len()..].copy_from_slice(pushval);
            U256::from_big_endian(&padded)
        }
    });
    LEN
}

#[inline]
pub(crate) unsafe fn dup<const HEIGHT: usize>(stack: &mut Stack) {
    stack.push(*stack.get(HEIGHT - 1));
}

#[inline]
pub(crate) unsafe fn swap<const HEIGHT: usize>(stack: &mut Stack) {
    stack.swap_top(HEIGHT);
}

#[inline]
pub(crate) unsafe fn pop(stack: &mut Stack) {
    stack.pop();
}
