use {crate::interp::stack::Stack, crate::interp::U256};

#[inline]
pub(crate) fn push<const LEN: usize>(stack: &mut Stack, code: &[u8]) -> usize {
    stack.push(U256::from_big_endian(&code[..LEN]));
    LEN
}

#[inline]
pub(crate) fn push1(stack: &mut Stack, v: u8) -> usize {
    stack.push(v.into());
    1
}

#[inline]
pub(crate) fn push32(stack: &mut Stack, code: &[u8]) -> usize {
    stack.push(U256::from_big_endian(&code[0..32]));
    32
}

#[inline]
pub(crate) fn dup<const HEIGHT: usize>(stack: &mut Stack) {
    stack.push(*stack.get(HEIGHT - 1));
}

#[inline]
pub(crate) fn swap<const HEIGHT: usize>(stack: &mut Stack) {
    stack.swap_top(HEIGHT);
}

#[inline]
pub(crate) fn pop(stack: &mut Stack) {
    stack.pop();
}
