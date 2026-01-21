use fil_actors_evm_shared::uints::U256;

#[inline]
pub fn byte(i: U256, x: U256) -> U256 {
    if i >= 32 { U256::ZERO } else { U256::from_u64(x.byte(31 - i.low_u64() as usize) as u64) }
}

#[inline]
pub fn shl(shift: U256, value: U256) -> U256 {
    if value.is_zero() || shift >= 256 { U256::ZERO } else { value << shift }
}

#[inline]
pub fn shr(shift: U256, value: U256) -> U256 {
    if value.is_zero() || shift >= 256 { U256::ZERO } else { value >> shift }
}

#[inline]
pub fn sar(shift: U256, mut value: U256) -> U256 {
    let negative = value.i256_is_negative();
    if negative {
        value = value.i256_neg();
    }

    if value.is_zero() || shift >= 256 {
        if negative {
            // value is < 0, pushing U256::MAX (== -1)
            U256::MAX
        } else {
            // value is >= 0, pushing 0
            U256::ZERO
        }
    } else {
        let shift = shift.low_u32();

        if negative {
            let shifted =
                (value.overflowing_sub(U256::ONE).0 >> shift).overflowing_add(U256::ONE).0;
            shifted.i256_neg()
        } else {
            value >> shift
        }
    }
}

#[inline]
pub fn clz(value: U256) -> U256 {
    U256::from(value.leading_zeros())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::interpreter::opcodes;
    use crate::interpreter::{Output, execution::Machine, system::System};
    use crate::{Bytecode, EthAddress, ExecutionState};
    use fil_actors_runtime::test_utils::MockRuntime;
    use fvm_shared::econ::TokenAmount;

    #[test]
    fn test_clz_eip7939_vectors_unit() {
        // Directly matches the EIP-7939 test cases.
        assert_eq!(clz(U256::ZERO), U256::from(256));
        assert_eq!(clz(U256::ONE << 255), U256::ZERO);
        assert_eq!(clz(U256::MAX), U256::ZERO);
        assert_eq!(clz(U256::ONE << 254), U256::ONE);
        assert_eq!(clz((U256::ONE << 255) - U256::ONE), U256::ONE);
        assert_eq!(clz(U256::ONE), U256::from(255));
    }

    #[test]
    fn test_clz_misc_unit() {
        // 2 is 10 binary, so 256 - 2 = 254
        assert_eq!(clz(U256::from(2)), U256::from(254));
    }

    fn clz_via_evm(value: U256) -> U256 {
        let rt = MockRuntime::default();
        rt.in_call.replace(true);

        let mut state = ExecutionState::new(
            EthAddress::from_id(1000),
            EthAddress::from_id(1000),
            TokenAmount::from_atto(0),
            Vec::new(),
        );

        let mut imm = [0u8; 32];
        value.write_as_big_endian(&mut imm);

        let mut code = Vec::with_capacity(1 + 32 + 1);
        code.push(opcodes::PUSH32);
        code.extend_from_slice(&imm);
        code.push(opcodes::CLZ);

        let mut system = System::new(&rt, false);
        let bytecode = Bytecode::new(code);
        let mut machine = Machine {
            system: &mut system,
            state: &mut state,
            bytecode: &bytecode,
            pc: 0,
            output: Output::default(),
        };

        machine.step().expect("PUSH32 step failed");
        machine.step().expect("CLZ step failed");
        machine.state.stack.pop_many::<1>().expect("missing CLZ result")[0]
    }

    #[test]
    fn test_clz_eip7939_vectors() {
        // From EIP-7939 test cases.
        assert_eq!(clz_via_evm(U256::ZERO), U256::from(256));
        assert_eq!(clz_via_evm(U256::ONE << 255), U256::ZERO);
        assert_eq!(clz_via_evm(U256::MAX), U256::ZERO);
        assert_eq!(clz_via_evm(U256::ONE << 254), U256::ONE);
        assert_eq!(clz_via_evm((U256::ONE << 255) - U256::ONE), U256::ONE);
        assert_eq!(clz_via_evm(U256::ONE), U256::from(255));
    }

    #[test]
    fn test_shl() {
        // Basic shift
        assert_eq!(shl(U256::from(2), U256::from(13)), U256::from(52));

        // 0/1 shifts.
        assert_eq!(shl(U256::ONE, U256::ONE), U256::from(2));
        assert_eq!(shl(U256::ONE, U256::ZERO), U256::ZERO);
        assert_eq!(shl(U256::ZERO, U256::ONE), U256::ONE);
        assert_eq!(shl(U256::ZERO, U256::ZERO), U256::ZERO);

        // shift max bits
        assert_eq!(shl(U256::ONE, U256::MAX), U256::MAX - U256::ONE);
        assert_eq!(shl(U256::from(2), U256::MAX), U256::MAX - U256::from(3));

        // shift by max
        assert_eq!(shl(U256::from(255), U256::MAX), U256::from_u128_words(i128::MIN as u128, 0));
        assert_eq!(shl(U256::from(256), U256::MAX), U256::ZERO);
        assert_eq!(shl(U256::from(257), U256::MAX), U256::ZERO);
    }

    #[test]
    fn test_shr() {
        // Basic shift
        assert_eq!(shr(U256::from(2), U256::from(13)), U256::from(3));

        // 0/1 shifts.
        assert_eq!(shr(U256::ONE, U256::ONE), U256::ZERO);
        assert_eq!(shr(U256::ONE, U256::ZERO), U256::ZERO);
        assert_eq!(shr(U256::ZERO, U256::ONE), U256::ONE);
        assert_eq!(shr(U256::ZERO, U256::ZERO), U256::ZERO);

        // shift max
        assert_eq!(shr(U256::from(255), U256::MAX), U256::ONE);
        assert_eq!(shr(U256::from(256), U256::MAX), U256::ZERO);
        assert_eq!(shr(U256::from(257), U256::MAX), U256::ZERO);
    }

    #[test]
    fn test_sar() {
        let pos_max = shr(U256::ONE, U256::MAX);

        // Basic shift
        assert_eq!(sar(U256::from(2), U256::from(13)), U256::from(3));
        assert_eq!(sar(U256::from(2), U256::from(13).i256_neg()), U256::from(4).i256_neg());

        // 0/1 shifts.
        assert_eq!(sar(U256::ONE, U256::ONE), U256::ZERO);
        assert_eq!(sar(U256::ONE, U256::ZERO), U256::ZERO);
        assert_eq!(sar(U256::ZERO, U256::ONE), U256::ONE);
        assert_eq!(sar(U256::ZERO, U256::ZERO), U256::ZERO);

        // shift max negative
        assert_eq!(sar(U256::from(255), U256::MAX), U256::MAX); // sign extends.
        assert_eq!(sar(U256::from(256), U256::MAX), U256::MAX);
        assert_eq!(sar(U256::from(257), U256::MAX), U256::MAX);

        // shift max positive.
        assert_eq!(sar(U256::from(254), pos_max), U256::ONE);
        assert_eq!(sar(U256::from(255), pos_max), U256::ZERO);
        assert_eq!(sar(U256::from(256), pos_max), U256::ZERO);
        assert_eq!(sar(U256::from(257), pos_max), U256::ZERO);
    }

    #[test]
    fn test_instruction_byte() {
        let value = U256::from_big_endian(&(1u8..=32u8).map(|x| 5 * x).collect::<Vec<u8>>());

        for i in 0u16..32 {
            let result = byte(U256::from(i), value);

            assert_eq!(result, U256::from(5 * (i + 1)));
        }

        let result = byte(U256::from(100u128), value);
        assert_eq!(result, U256::zero());

        let result = byte(U256::from_u128_words(1, 0), value);
        assert_eq!(result, U256::zero());
    }
}
