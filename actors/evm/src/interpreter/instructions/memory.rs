#!allow[clippy::result-unit-err]

use fil_actors_runtime::{ActorError, AsActorError};

use crate::{EVM_CONTRACT_ILLEGAL_MEMORY_ACCESS, EVM_WORD_SIZE};

use {
    crate::interpreter::memory::Memory,
    crate::interpreter::{ExecutionState, System, U256},
    fil_actors_runtime::runtime::Runtime,
    std::num::NonZeroUsize,
};

#[derive(Debug)]
pub struct MemoryRegion {
    pub offset: usize,
    pub size: NonZeroUsize,
}

#[inline]
pub fn get_memory_region(
    mem: &mut Memory,
    offset: impl TryInto<u32>,
    size: impl TryInto<u32>,
) -> Result<Option<MemoryRegion>, ActorError> {
    // We use u32 because we don't support more than 4GiB of memory anyways.
    // Also, explicitly check math so we don't panic and/or wrap around.
    let size: u32 = size.try_into().map_err(|_| {
        ActorError::unchecked(
            EVM_CONTRACT_ILLEGAL_MEMORY_ACCESS,
            "size must be less than max u32".into(),
        )
    })?;
    if size == 0 {
        return Ok(None);
    }
    let offset: u32 = offset.try_into().map_err(|_| {
        ActorError::unchecked(
            EVM_CONTRACT_ILLEGAL_MEMORY_ACCESS,
            "offset must be less than max u32".into(),
        )
    })?;
    let new_size: u32 = offset
        .checked_add(size)
        .context_code(EVM_CONTRACT_ILLEGAL_MEMORY_ACCESS, "new memory size exceeds max u32")?;

    mem.grow(new_size as usize);

    Ok(Some(MemoryRegion {
        offset: offset as usize,
        size: unsafe { NonZeroUsize::new_unchecked(size as usize) },
    }))
}

pub fn copy_to_memory(
    memory: &mut Memory,
    dest_offset: U256,
    dest_size: U256,
    data_offset: U256,
    data: &[u8],
    zero_fill: bool,
) -> Result<(), ActorError> {
    let region = get_memory_region(memory, dest_offset, dest_size)?;

    #[inline(always)]
    fn min(a: U256, b: usize) -> usize {
        if a < (b as u64) {
            a.low_u64() as usize
        } else {
            b
        }
    }

    if let Some(region) = &region {
        let data_len = data.len();
        let data_offset = min(data_offset, data_len);
        let copy_size = min(dest_size, data_len - data_offset);

        if copy_size > 0 {
            memory[region.offset..region.offset + copy_size]
                .copy_from_slice(&data[data_offset..data_offset + copy_size]);
        }

        if zero_fill && region.size.get() > copy_size {
            memory[region.offset + copy_size..region.offset + region.size.get()].fill(0);
        }
    }

    Ok(())
}

#[inline]
pub fn mload(
    state: &mut ExecutionState,
    _system: &System<impl Runtime>,
    index: U256,
) -> Result<U256, ActorError> {
    let region = get_memory_region(&mut state.memory, index, EVM_WORD_SIZE)?.expect("empty region");
    let value =
        U256::from_big_endian(&state.memory[region.offset..region.offset + region.size.get()]);

    Ok(value)
}

#[inline]
pub fn mstore(
    state: &mut ExecutionState,
    _system: &System<impl Runtime>,
    index: U256,
    value: U256,
) -> Result<(), ActorError> {
    let region = get_memory_region(&mut state.memory, index, EVM_WORD_SIZE)?.expect("empty region");

    let mut bytes = [0u8; EVM_WORD_SIZE];
    value.to_big_endian(&mut bytes);
    state.memory[region.offset..region.offset + EVM_WORD_SIZE].copy_from_slice(&bytes);

    Ok(())
}

#[inline]
pub fn mstore8(
    state: &mut ExecutionState,
    _system: &System<impl Runtime>,
    index: U256,
    value: U256,
) -> Result<(), ActorError> {
    let region = get_memory_region(&mut state.memory, index, 1)?.expect("empty region");

    let value = (value.low_u32() & 0xff) as u8;
    state.memory[region.offset] = value;

    Ok(())
}

#[inline]
pub fn msize(
    state: &mut ExecutionState,
    _system: &System<impl Runtime>,
) -> Result<U256, ActorError> {
    Ok(u64::try_from(state.memory.len()).unwrap().into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{evm_unit_test, interpreter::memory::Memory};

    #[test]
    fn copy_to_memory_big() {
        let mut mem: Memory = Default::default();
        let result = copy_to_memory(
            &mut mem,
            U256::zero(),
            U256::from(1u128 << 40),
            U256::zero(),
            &[],
            true,
        );
        assert_eq!(result.unwrap_err().exit_code(), EVM_CONTRACT_ILLEGAL_MEMORY_ACCESS);
    }

    #[test]
    fn copy_to_memory_zero() {
        let mut mem: Memory = Default::default();
        let result = copy_to_memory(
            &mut mem,
            U256::zero(),
            U256::zero(),
            U256::zero(),
            &[1u8, 2u8, 3u8],
            true,
        );
        assert_eq!(result, Ok(()));
        assert!(mem.is_empty());
    }

    #[test]
    fn copy_to_memory_some() {
        let data = &[1u8, 2u8, 3u8];
        let mut mem: Memory = Default::default();
        let result =
            copy_to_memory(&mut mem, U256::zero(), U256::from(3), U256::zero(), data, true);
        assert_eq!(result, Ok(()));
        assert_eq!(mem.len(), 32);
        assert_eq!(&mem[0..3], data);
    }

    #[test]
    fn copy_to_memory_some_truncate() {
        let data = &[1u8, 2u8, 3u8, 4u8];
        let result_data = &[1u8, 2u8, 3u8, 0u8];

        let mut mem: Memory = Default::default();
        let result =
            copy_to_memory(&mut mem, U256::zero(), U256::from(3), U256::zero(), data, true);
        assert_eq!(result, Ok(()));
        assert_eq!(mem.len(), 32);
        assert_eq!(&mem[0..4], result_data);
    }

    #[test]
    fn test_mload_nothing() {
        evm_unit_test! {
            (m) [
                PUSH0;
                MLOAD;
            ]

            m.step().expect("execution step failed");
            m.step().expect("execution step failed");

            assert_eq!(m.state.stack.len(), 1);
            assert_eq!(m.state.stack.pop().unwrap(), U256::zero());
        };
    }

    #[test]
    fn test_mload_large_offset() {
        evm_unit_test! {
            (m) [
                PUSH4; // garbage offset
                0x01;
                0x02;
                0x03;
                0x04;
                MLOAD;
            ]

            m.step().expect("execution step failed");
            m.step().expect("execution step failed");

            assert_eq!(m.state.stack.len(), 1);
            assert_eq!(m.state.stack.pop().unwrap(), U256::zero());
        };
    }

    #[test]
    fn test_mload_word() {
        for sh in 0..32 {
            evm_unit_test! {
                (m) [
                    PUSH1;
                    {sh};
                    MLOAD;
                ]

                m.state.memory.grow(32);
                m.state.memory[..32].copy_from_slice(&U256::MAX.to_bytes());

                m.step().expect("execution step failed");
                m.step().expect("execution step failed");

                assert_eq!(m.state.stack.len(), 1);
                assert_eq!(m.state.stack.pop().unwrap(), U256::MAX << (8 * sh));
            };
        }
    }

    #[test]
    fn test_mstore8_basic() {
        for i in 0..=u8::MAX {
            evm_unit_test! {
                (m) [
                    PUSH1;
                    {i};
                    PUSH0;
                    MSTORE8;
                ]
                m.step().expect("execution step failed");
                m.step().expect("execution step failed");
                m.step().expect("execution step failed");

                assert_eq!(m.state.stack.len(), 0);
                assert_eq!(m.state.memory[0], i);
            };
        }
    }

    #[test]
    fn test_mstore8_overwrite() {
        evm_unit_test! {
            (m) [
                PUSH1;
                0x01;
                PUSH1;
                0x01;
                MSTORE8;
            ]
            // index has garbage
            m.state.memory.grow(32);
            m.state.memory[0] = 0xab;
            m.state.memory[1] = 0xff;
            m.state.memory[2] = 0xfe;

            m.step().expect("execution step failed");
            m.step().expect("execution step failed");
            m.step().expect("execution step failed");

            assert_eq!(m.state.stack.len(), 0);
            // overwritten
            assert_eq!(m.state.memory[1], 0x01);
            // byte after isn't touched
            assert_eq!(m.state.memory[2], 0xfe);
            // byte before isn't touched
            assert_eq!(m.state.memory[0], 0xab);
        };
    }

    #[test]
    fn test_mstore8_large_offset() {
        for sh in 0..16 {
            let i = 1u16 << sh;
            let [a, b] = i.to_be_bytes();
            evm_unit_test! {
                (m) [
                    PUSH1;
                    0xff;
                    PUSH2;
                    {a};
                    {b};
                    MSTORE8;
                ]
                m.step().expect("execution step failed");
                m.step().expect("execution step failed");
                m.step().expect("execution step failed");

                assert_eq!(m.state.stack.len(), 0);
                assert_eq!(m.state.memory[i as usize], 0xff);
                // leading memory is zeroed
                assert_eq!(&m.state.memory[..i as usize], &vec![0; i as usize]);
            };
        }
    }

    #[test]
    fn test_mstore8_garbage() {
        evm_unit_test! {
            (m) [
                PUSH32;
                0xff;
                0xff;
                0xff;
                0xff;
                0xff;
                0xff;
                0xff;
                0xff;
                0xff;
                0xff;
                0xff;
                0xff;
                0xff;
                0xff;
                0xff;
                0xff;
                0xff;
                0xff;
                0xff;
                0xff;
                0xff;
                0xff;
                0xff;
                0xff;
                0xff;
                0xff;
                0xff;
                0xff;
                0xff;
                0xff;
                0xff;
                0x01;
                PUSH0;
                MSTORE8;
            ]
            m.step().expect("execution step failed");
            m.step().expect("execution step failed");
            m.step().expect("execution step failed");

            assert_eq!(m.state.stack.len(), 0);
            assert_eq!(m.state.memory[0], 0x01);
            // garbage is not written alongside byte
            assert_eq!(&m.state.memory[1..32], &[0; 31]);
        };
    }

    #[test]
    fn test_mstore_basic() {
        evm_unit_test! {
            (m) [
                PUSH2;
                0xff;
                0xfe;
                PUSH0;
                MSTORE;
            ]
            m.step().expect("execution step failed");
            m.step().expect("execution step failed");
            m.step().expect("execution step failed");

            assert_eq!(m.state.stack.len(), 0);
            assert_eq!(m.state.memory[30..32], [0xff, 0xfe]);
            // nothing else is written to memory
            assert_eq!(&m.state.memory[..30], &[0; 30]);
        };
    }

    #[test]
    fn test_mstore_overwrite() {
        evm_unit_test! {
            (m) [
                PUSH2;
                0xff;
                0xfe;
                PUSH0;
                MSTORE;
            ]
            m.state.memory.grow(64);
            m.state.memory[..EVM_WORD_SIZE].copy_from_slice(&[0xff; EVM_WORD_SIZE]);
            // single byte outside expected overwritten area
            m.state.memory[32] = 0xfe;

            m.step().expect("execution step failed");
            m.step().expect("execution step failed");
            m.step().expect("execution step failed");

            assert_eq!(m.state.stack.len(), 0);
            assert_eq!(m.state.memory[30..32], [0xff, 0xfe]);
            // zeroes fill remaining word
            assert_eq!(&m.state.memory[..30], &[0; 30]);
            // nothing written outside of word
            assert_eq!(m.state.memory[32], 0xfe);
        };
    }

    #[test]
    fn test_msize_multiple_mstore8() {
        evm_unit_test! {
            (m) [
                PUSH1;
                0xff;
                PUSH1;
                {42}; // offset of 42
                MSTORE8;
                MSIZE;
            ]

            m.step().expect("execution step failed");
            m.step().expect("execution step failed");
            m.step().expect("execution step failed");
            m.step().expect("execution step failed");

            assert_eq!(m.state.stack.len(), 1);
            // msize is always a multiple of 32
            assert_eq!(m.state.stack.pop().unwrap(), U256::from(64));
        };
    }

    #[test]
    fn test_msize_multiple_mstore() {
        evm_unit_test! {
            (m) [
                PUSH1;
                0xff;
                PUSH1;
                {12}; // offset of 12
                MSTORE;
                MSIZE;
            ]

            m.step().expect("execution step failed");
            m.step().expect("execution step failed");
            m.step().expect("execution step failed");
            m.step().expect("execution step failed");

            assert_eq!(m.state.stack.len(), 1);
            // 12 + 32 = 42, round up to nearest 32 = 64
            assert_eq!(m.state.stack.pop().unwrap(), U256::from(64));
        };
    }
    #[test]
    fn test_msize_basic() {
        // Demonstrate that MSIZE depends on memory.len()
        // Normally this should never happen and we wont panic from it.
        evm_unit_test! {
            (m) [
                MSIZE;
            ]

            m.state.memory.grow(12);
            m.step().expect("execution step failed");
            assert_eq!(m.state.stack.pop().unwrap(), U256::from(32));
        };
    }
}
