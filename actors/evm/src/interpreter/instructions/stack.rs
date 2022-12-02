#![allow(clippy::missing_safety_doc)]
use {crate::interpreter::stack::Stack, crate::interpreter::U256};

#[inline]
pub(crate) unsafe fn push<const LEN: usize>(stack: &mut Stack, code: &[u8]) -> usize {
    stack.push(if code.len() < LEN {
        let mut padded = [0; LEN];
        padded[..code.len()].copy_from_slice(code);
        U256::from_big_endian(&padded)
    } else {
        U256::from_big_endian(&code[..LEN])
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

#[test]
fn test_push_pad_right() {
    let mut stack = Stack::new();
    unsafe {
        assert!(stack.ensure(1));
        assert_eq!(push::<4>(&mut stack, &[0xde, 0xad]), 4);
        assert_eq!(stack.len(), 1);
        assert_eq!(stack.get(0), &U256::from(0xdead0000u64));
    }
}
