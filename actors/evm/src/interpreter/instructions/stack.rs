use {crate::interpreter::stack::Stack, crate::interpreter::U256, crate::interpreter::StatusCode};

#[inline]
pub fn push<const LEN: usize>(stack: &mut Stack, code: &[u8]) -> Result<usize, StatusCode> {
    let pushval = &code[..LEN];
    stack.push(match pushval.len() {
        0 => U256::zero(),
        32 => U256::from_big_endian(pushval),
        _ => {
            let mut padded = [0; 32];
            padded[32 - pushval.len()..].copy_from_slice(pushval);
            U256::from_big_endian(&padded)
        }
    })?;
    Ok(LEN)
}

#[inline]
pub fn dup<const HEIGHT: usize>(stack: &mut Stack) -> Result<(), StatusCode> {
    stack.push(stack.peek(HEIGHT - 1)?)
}

#[inline]
pub fn swap<const HEIGHT: usize>(stack: &mut Stack) -> Result<(), StatusCode> {
    stack.swap(HEIGHT)
}

#[inline]
pub fn pop(stack: &mut Stack) -> Result<(), StatusCode> {
    stack.pop()?;
    Ok(())
}
