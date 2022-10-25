use {crate::interpreter::stack::Stack, crate::interpreter::uints::*, crate::interpreter::U256};

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
    if !b.is_zero() {
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
    if !b.is_zero() {
        *b = a % *b;
    }
}

#[inline]
pub fn smod(stack: &mut Stack) {
    let a = stack.pop();
    let b = stack.get_mut(0);

    *b = i256_mod(a, *b);
}

#[inline]
pub fn addmod(stack: &mut Stack) {
    let a = stack.pop();
    let b = stack.pop();
    let c = stack.get_mut(0);

    if !c.is_zero() {
        let al: U512 = a.into();
        let bl: U512 = b.into();
        let cl: U512 = (*c).into();

        *c = (al + bl % cl).low_u256();
    }
}

#[inline]
pub fn mulmod(stack: &mut Stack) {
    let a = stack.pop();
    let b = stack.pop();
    let c = stack.get_mut(0);

    if !c.is_zero() {
        let al: U512 = a.into();
        let bl: U512 = b.into();
        let cl: U512 = (*c).into();
        *c = (al * bl % cl).low_u256();
    }
}

#[inline]
pub fn signextend(stack: &mut Stack) {
    let a = stack.pop();
    let b = stack.get_mut(0);

    if a < U256::from_u64(32) {
        let bit_index = 8 * a.low_u64() + 7;
        let mask = (U256::ONE << bit_index) - U256::ONE;
        *b = if b.bit(bit_index as usize) { *b | !mask } else { *b & mask }
    }
}

#[inline]
pub fn exp(stack: &mut Stack) {
    let mut base = stack.pop();
    let mut power = stack.pop();

    let mut v = U256::ONE;

    // TODO: avoid the shift here to make it even faster.
    while !power.is_zero() {
        if !power.is_even() {
            v = v.overflowing_mul(base).0;
            // Subtracts one when odd, and is much faster than real subtraction here.
            power.clear_low_bit();
        }
        power >>= 1;
        base = base.overflowing_mul(base).0;
    }

    stack.push(v);
}
