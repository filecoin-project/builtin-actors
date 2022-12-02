#![allow(clippy::missing_safety_doc)]

use crate::interpreter::StatusCode;
use {crate::interpreter::stack::Stack, crate::interpreter::U256};

#[inline]
pub(crate) fn push<const LEN: usize>(stack: &mut Stack, code: &[u8]) -> Result<usize, StatusCode> {
    stack
        .push(if code.len() < LEN {
            let mut padded = [0; LEN];
            padded[..code.len()].copy_from_slice(code);
            U256::from_big_endian(&padded)
        } else {
            U256::from_big_endian(&code[..LEN])
        })
        .map(|_| LEN)
}

#[inline]
pub(crate) fn dup<const HEIGHT: usize>(stack: &mut Stack) -> Result<(), StatusCode> {
    stack.dup(HEIGHT)
}

#[inline]
pub(crate) fn swap<const HEIGHT: usize>(stack: &mut Stack) -> Result<(), StatusCode> {
    stack.swap_top(HEIGHT)
}

#[inline]
pub(crate) fn pop(stack: &mut Stack) -> Result<(), StatusCode> {
    stack.drop()
}

#[test]
fn test_push_pad_right() {
    let mut stack = Stack::new();
    assert_eq!(push::<4>(&mut stack, &[0xde, 0xad]).unwrap(), 4);
    assert_eq!(stack.len(), 1);
    assert_eq!(stack.pop().unwrap(), U256::from(0xdead0000u64));
}
