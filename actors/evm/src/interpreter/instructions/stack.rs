#![allow(clippy::missing_safety_doc)]

use crate::interpreter::StatusCode;
use {crate::interpreter::stack::Stack, crate::interpreter::U256};

macro_rules! be_u64 {
    ($byte:expr) => {$byte as u64};
    ($byte1:expr, $byte2:expr $(,$rest:expr)*) => {
        be_u64!{((($byte1 as u64) << 8) | ($byte2 as u64)) $(,$rest)*}
    };
}

#[inline]
pub(crate) fn push<const LEN: usize>(stack: &mut Stack, code: &[u8]) -> Result<usize, StatusCode> {
    if code.len() < LEN {
        // this is a pathological edge case, as the contract will immediately stop execution.
        // we still handle it for correctness sake (and obviously avoid crashing out of bounds)
        let mut padded = [0; LEN];
        padded[..code.len()].copy_from_slice(code);
        stack.push(U256::from_big_endian(&padded))?;
    } else {
        stack.push(match LEN {
            // explicitly unroll up to u64 (single limb) pushes
            1 => U256::from_u64(be_u64! {code[0]}),
            2 => U256::from_u64(be_u64! {code[0], code[1]}),
            3 => U256::from_u64(be_u64! {code[0], code[1], code[2]}),
            4 => U256::from_u64(be_u64! {code[0], code[1], code[2], code[3]}),
            5 => U256::from_u64(be_u64! {code[0], code[1], code[2], code[3], code[4]}),
            6 => U256::from_u64(be_u64! {code[0], code[1], code[2], code[3], code[4], code[5]}),
            7 => U256::from_u64(
                be_u64! {code[0], code[1], code[2], code[3], code[4], code[5], code[6]},
            ),
            8 => U256::from_u64(
                be_u64! {code[0], code[1], code[2], code[3], code[4], code[5], code[6], code[7]},
            ),
            _ => U256::from_big_endian(&code[..LEN]),
        })?;
    }
    Ok(LEN)
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
