use {
    crate::interpreter::output::StatusCode, crate::interpreter::stack::Stack,
    crate::interpreter::uints::*, crate::interpreter::ExecutionState, crate::interpreter::U256,
};

#[inline]
pub fn add(stack: &mut Stack) {
    let a = stack.pop();
    let b = stack.pop();
    stack.push(a.overflowing_add(b).0);
}

#[inline]
pub fn mul(stack: &mut Stack) {
    let a = stack.pop();
    let b = stack.pop();
    stack.push(a.overflowing_mul(b).0);
}

#[inline]
pub fn sub(stack: &mut Stack) {
    let a = stack.pop();
    let b = stack.pop();
    stack.push(a.overflowing_sub(b).0);
}

#[inline]
pub fn div(stack: &mut Stack) {
    let a = stack.pop();
    let b = stack.get_mut(0);
    if *b == U256::zero() {
        *b = U256::zero()
    } else {
        *b = a / *b
    }
}

#[inline]
pub fn sdiv(stack: &mut Stack) {
    let a = stack.pop();
    let b = stack.pop();
    let v = i256_div(a, b);
    stack.push(v);
}

#[inline]
pub fn modulo(stack: &mut Stack) {
    let a = stack.pop();
    let b = stack.get_mut(0);
    *b = if *b == U256::zero() { U256::zero() } else { a % *b };
}

#[inline]
pub fn smod(stack: &mut Stack) {
    let a = stack.pop();
    let b = stack.get_mut(0);

    if *b == U256::zero() {
        *b = U256::zero()
    } else {
        *b = i256_mod(a, *b);
    };
}

#[inline]
pub fn addmod(stack: &mut Stack) {
    let a = stack.pop();
    let b = stack.pop();
    let c = stack.pop();

    let v = if c == U256::zero() {
        U256::zero()
    } else {
        let mut a_be = [0u8; 32];
        let mut b_be = [0u8; 32];
        let mut c_be = [0u8; 32];

        a.to_big_endian(&mut a_be);
        b.to_big_endian(&mut b_be);
        c.to_big_endian(&mut c_be);

        let a = U512::from_big_endian(&a_be);
        let b = U512::from_big_endian(&b_be);
        let c = U512::from_big_endian(&c_be);

        let v = a + b % c;
        let mut v_be = [0u8; 64];
        v.to_big_endian(&mut v_be);
        U256::from_big_endian(&v_be)
    };

    stack.push(v);
}

#[inline]
pub fn mulmod(stack: &mut Stack) {
    let a = stack.pop();
    let b = stack.pop();
    let c = stack.pop();

    let v = if c == U256::zero() {
        U256::zero()
    } else {
        let mut a_be = [0u8; 32];
        let mut b_be = [0u8; 32];
        let mut c_be = [0u8; 32];

        a.to_big_endian(&mut a_be);
        b.to_big_endian(&mut b_be);
        c.to_big_endian(&mut c_be);

        let a = U512::from_big_endian(&a_be);
        let b = U512::from_big_endian(&b_be);
        let c = U512::from_big_endian(&c_be);

        let v = a * b % c;
        let mut v_be = [0u8; 64];
        v.to_big_endian(&mut v_be);
        U256::from_big_endian(&v_be)
    };

    stack.push(v);
}

#[inline]
pub fn signextend(stack: &mut Stack) {
    let a = stack.pop();
    let b = stack.get_mut(0);

    if a < U256::from(32) {
        let bit_index = (8 * u256_low(a) as u8 + 7) as u16;
        let hi = u256_high(*b);
        let lo = u256_low(*b);
        let bit = if bit_index > 0x7f { hi } else { lo } & (1 << (bit_index % 128)) != 0;
        let mask = (U256::from(1) << bit_index) - U256::from(1);
        *b = if bit { *b | !mask } else { *b & mask }
    }
}

#[inline]
pub fn exp(state: &mut ExecutionState) -> Result<(), StatusCode> {
    let mut base = state.stack.pop();
    let mut power = state.stack.pop();

    if power > U256::zero() {
        let factor = 50;
        let additional_gas = factor * (log2floor(power) / 8 + 1);
        state.gas_left -= additional_gas as i64;
        if state.gas_left < 0 {
            return Err(StatusCode::OutOfGas);
        }
    }

    let mut v = U256::from(1);

    while power > U256::zero() {
        if (power & U256::from(1)) != U256::zero() {
            v = v.overflowing_mul(base).0;
        }
        power >>= 1;
        base = base.overflowing_mul(base).0;
    }

    state.stack.push(v);

    Ok(())
}
