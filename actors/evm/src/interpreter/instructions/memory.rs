#!allow[clippy::result-unit-err]

use fil_actors_evm_shared::uints::U256;
use fil_actors_runtime::{ActorError, AsActorError};

use crate::{EVM_CONTRACT_ILLEGAL_MEMORY_ACCESS, EVM_WORD_SIZE};

use {
    crate::interpreter::memory::Memory,
    crate::interpreter::{ExecutionState, System},
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

#[inline]
pub fn mcopy(
    state: &mut ExecutionState,
    _: &System<impl Runtime>,
    dest_index: U256,
    src_index: U256,
    size: U256,
) -> Result<(), ActorError> {
    // We are copying between two potentially overlapping slices in the same memory.
    // MCOPY spec: Copying takes place as if an intermediate buffer was used,
    //             allowing the destination and source to overlap.

    // expand memory to accomodate requested src_index + size
    let region = get_memory_region(&mut state.memory, src_index, size)?.expect("empty region");
    let memory_slice = state.memory[region.offset..region.offset + region.size.get()].to_vec();

    // expand memory to match dest_index + size
    let _destination_region =
        get_memory_region(&mut state.memory, dest_index, size)?.expect("empty region");

    //copy
    copy_to_memory(&mut state.memory, dest_index, size, U256::zero(), &memory_slice, true)
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
    fn test_mcopy() {
        const LENGTH: usize = 2;
        const OFFSET: usize = 1;
        const DEST_OFFSET: usize = 0;

        evm_unit_test! {
            (m) {
                MCOPY;
            }

            // Grow memory and set initial values
            m.state.memory.grow(32);
            m.state.memory[..3].copy_from_slice(&[0x00, 0x01, 0x02]);

            // Set up stack
            m.state.stack.push(U256::from(LENGTH)).unwrap();
            m.state.stack.push(U256::from(OFFSET)).unwrap();
            m.state.stack.push(U256::from(DEST_OFFSET)).unwrap();

            // Execute and assert
            assert!(m.step().is_ok(), "execution step failed");
            assert_eq!(m.state.stack.len(), 0);

            // Setup expected memory and assert
            let mut expected = [0u8; 32];
            expected[..3].copy_from_slice(&[0x01, 0x02, 0x02]);
            assert_eq!(&*m.state.memory, &expected);
        };
    }

    #[test]
    fn test_mcopy_0_32_32() {
        evm_unit_test! {
            (m) {
                MCOPY;
            }

            // Grow memory and set initial values
            m.state.memory.grow(64);
            m.state.memory[32..64].copy_from_slice(&[
                0x00, 0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, 0x09, 0x0a, 0x0b, 0x0c, 0x0d, 0x0e, 0x0f,
                0x10, 0x11, 0x12, 0x13, 0x14, 0x15, 0x16, 0x17, 0x18, 0x19, 0x1a, 0x1b, 0x1c, 0x1d, 0x1e, 0x1f
            ]);

            // Set up stack
            m.state.stack.push(U256::from(32)).unwrap();  // length
            m.state.stack.push(U256::from(32)).unwrap();  // source offset
            m.state.stack.push(U256::from(0)).unwrap();   // destination offset

            // Execute and assert
            assert!(m.step().is_ok(), "execution step failed");
            assert_eq!(m.state.stack.len(), 0);

            // Setup expected memory and assert
            let mut expected = [0u8; 64];
            expected[0..64].copy_from_slice(&[
                0x00, 0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, 0x09, 0x0a, 0x0b, 0x0c, 0x0d, 0x0e, 0x0f,
                0x10, 0x11, 0x12, 0x13, 0x14, 0x15, 0x16, 0x17, 0x18, 0x19, 0x1a, 0x1b, 0x1c, 0x1d, 0x1e, 0x1f,
                0x00, 0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, 0x09, 0x0a, 0x0b, 0x0c, 0x0d, 0x0e, 0x0f,
                0x10, 0x11, 0x12, 0x13, 0x14, 0x15, 0x16, 0x17, 0x18, 0x19, 0x1a, 0x1b, 0x1c, 0x1d, 0x1e, 0x1f
            ]);
            assert_eq!(&*m.state.memory, &expected);
        };
    }

    #[test]
    fn test_mcopy_0_0_32() {
        evm_unit_test! {
            (m) {
                MCOPY;
            }

            // Grow memory and set initial values
            m.state.memory.grow(32);
            m.state.memory[..32].copy_from_slice(&[0x01; 32]);

            // Set up stack
            m.state.stack.push(U256::from(32)).unwrap();  // length
            m.state.stack.push(U256::from(0)).unwrap();   // source offset
            m.state.stack.push(U256::from(0)).unwrap();   // destination offset

            // Execute and assert
            assert!(m.step().is_ok(), "execution step failed");
            assert_eq!(m.state.stack.len(), 0);

            // Setup expected memory and assert
            let expected = [0x01; 32];
            assert_eq!(&m.state.memory[..32], &expected);
        };
    }

    #[test]
    fn test_mcopy_0_1_8() {
        evm_unit_test! {
            (m) {
                MCOPY;
            }

            // Grow memory and set initial values
            m.state.memory.grow(32);
            m.state.memory[..8].copy_from_slice(&[0x00, 0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07]);
            m.state.memory[8] = 0x08;

            // Set up stack
            m.state.stack.push(U256::from(8)).unwrap();   // length
            m.state.stack.push(U256::from(1)).unwrap();   // source offset
            m.state.stack.push(U256::from(0)).unwrap();   // destination offset

            // Execute and assert
            assert!(m.step().is_ok(), "execution step failed");
            assert_eq!(m.state.stack.len(), 0);

            // Setup expected memory and assert
            let mut expected = [0u8; 32];
            expected[..8].copy_from_slice(&[0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08]);
            expected[8] = 0x08;
            assert_eq!(&m.state.memory[..9], &expected[..9]);
        };
    }

    #[test]
    fn test_mcopy_1_0_8() {
        evm_unit_test! {
            (m) {
                MCOPY;
            }

            // Grow memory and set initial values
            m.state.memory.grow(32);
            m.state.memory[..8].copy_from_slice(&[0x00, 0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07]);
            m.state.memory[8] = 0x08;

            // Set up stack
            m.state.stack.push(U256::from(8)).unwrap();   // length
            m.state.stack.push(U256::from(0)).unwrap();   // source offset
            m.state.stack.push(U256::from(1)).unwrap();   // destination offset

            // Execute and assert
            assert!(m.step().is_ok(), "execution step failed");
            assert_eq!(m.state.stack.len(), 0);

            // Setup expected memory and assert
            let mut expected = [0u8; 32];
            expected[..8].copy_from_slice(&[0x00, 0x00, 0x01, 0x02, 0x03, 0x04, 0x05, 0x06]);
            expected[8] = 0x07;
            assert_eq!(&m.state.memory[..9], &expected[..9]);
        };
    }

    #[test]
    fn test_mcopy_out_of_range_dest() {
        evm_unit_test! {
            (m) {
                MCOPY;
            }

            // Initial memory setup
            m.state.memory.grow(32);
            m.state.memory[..4].copy_from_slice(&[0x01, 0x02, 0x03, 0x04]);

            // Set up stack: Attempt to copy to a destination beyond the current memory range
            m.state.stack.push(U256::from(4)).unwrap();  // length
            m.state.stack.push(U256::from(0)).unwrap();  // source offset
            m.state.stack.push(U256::from(64)).unwrap(); // out of range destination offset

            // Execute and expect memory expansion
            assert!(m.step().is_ok(), "execution step failed");
            assert_eq!(m.state.stack.len(), 0);

            // Check that memory was expanded correctly
            assert_eq!(m.state.memory.len(), 96);

            // Check the memory contents
            let mut expected = [0u8; 96];
            expected[..4].copy_from_slice(&[0x01, 0x02, 0x03, 0x04]);
            expected[64..68].copy_from_slice(&[0x01, 0x02, 0x03, 0x04]);
            assert_eq!(&*m.state.memory, &expected[..]);
        };
    }

    #[test]
    fn test_mcopy_partially_out_of_range_source() {
        evm_unit_test! {
            (m) {
                MCOPY;
            }

            // Initial memory setup
            m.state.memory.grow(32);
            m.state.memory[..28].copy_from_slice(&[0x01; 28]);

            // Set up stack: Source partially out of range
            m.state.stack.push(U256::from(10)).unwrap();  // length
            m.state.stack.push(U256::from(24)).unwrap(); // source offset (partially out of range)
            m.state.stack.push(U256::from(0)).unwrap();  // destination offset

            // Execute and expect memory expansion
            assert!(m.step().is_ok(), "execution step failed");
            assert_eq!(m.state.stack.len(), 0);

            // Check the length of the memory after the operation
            assert_eq!(m.state.memory.len(), 32+EVM_WORD_SIZE);  // Memory should remain at 32 bytes after the operation

            // Check that memory was expanded correctly
            let mut expected = vec![0x01; 4];  // First 4 bytes copied
            expected.extend_from_slice(&[0x00; 4]);  // Remaining 4 bytes unchanged
            assert_eq!(&m.state.memory[..8], &expected[..8]);
        };
    }

    #[test]
    fn test_mcopy_fully_out_of_range_dest_fails() {
        evm_unit_test! {
            (m) {
                MCOPY;
            }

            // Initial memory setup
            m.state.memory.grow(32);
            m.state.memory[..4].copy_from_slice(&[0x01, 0x02, 0x03, 0x04]);

            // Set up stack: Attempt to copy to a destination fully out of range
            m.state.stack.push(U256::from(4)).unwrap();  // length
            m.state.stack.push(U256::from(0)).unwrap();  // source offset
            m.state.stack.push(U256::from(128)).unwrap(); // fully out of range destination offset


            // Execute and assert memory grows
            assert!(m.step().is_ok(), "expected step to succeed and grow memory");
            assert_eq!(m.state.memory.len(), 160);  // Expected memory to grow

            // Check the memory contents
            let mut expected = [0u8; 132];
            expected[..4].copy_from_slice(&[0x01, 0x02, 0x03, 0x04]);
            expected[128..132].copy_from_slice(&[0x01, 0x02, 0x03, 0x04]);
            assert_eq!(&m.state.memory[0..132], &expected[0..132]);

        };
    }

    #[test]
    fn test_mload_nothing() {
        evm_unit_test! {
            (m) {
                PUSH0;
                MLOAD;
            }

            m.step().expect("execution step failed");
            m.step().expect("execution step failed");

            assert_eq!(m.state.stack.len(), 1);
            assert_eq!(m.state.stack.pop().unwrap(), U256::zero());
        };
    }

    #[test]
    fn test_mload_large_offset() {
        evm_unit_test! {
            (m) {
                PUSH4; // garbage offset
                0x01;
                0x02;
                0x03;
                0x04;
                MLOAD;
            }

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
                (m) {
                    PUSH1;
                    {sh};
                    MLOAD;
                }

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
                (m) {
                    PUSH1;
                    {i};
                    PUSH0;
                    MSTORE8;
                }
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
            (m) {
                PUSH1;
                0x01;
                PUSH1;
                0x01;
                MSTORE8;
            }
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
                (m) {
                    PUSH1;
                    0xff;
                    PUSH2;
                    {a};
                    {b};
                    MSTORE8;
                }
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
            (m) {
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
            }
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
            (m) {
                PUSH2;
                0xff;
                0xfe;
                PUSH0;
                MSTORE;
            }
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
            (m) {
                PUSH2;
                0xff;
                0xfe;
                PUSH0;
                MSTORE;
            }
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
            (m) {
                PUSH1;
                0xff;
                PUSH1;
                {42}; // offset of 42
                MSTORE8;
                MSIZE;
            }

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
            (m) {
                PUSH1;
                0xff;
                PUSH1;
                {12}; // offset of 12
                MSTORE;
                MSIZE;
            }

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
            (m) {
                MSIZE;
            }

            m.state.memory.grow(12);
            m.step().expect("execution step failed");
            assert_eq!(m.state.stack.pop().unwrap(), U256::from(32));
        };
    }

    macro_rules! check_mem {
        ($mem:ident, $region:ident, $len:expr) => {
            match $region {
                Some(MemoryRegion { offset, size }) => {
                    let sizeu: usize = size.into();
                    assert!(sizeu == $len);
                    assert!($mem.len() >= offset + sizeu);
                    for x in offset..offset + sizeu {
                        assert_eq!($mem[x], 0);
                    }
                }
                None => {
                    panic!("no memory region");
                }
            }
        };
    }

    #[test]
    fn test_memread_simple() {
        // simple read in bounds
        let mut mem = Memory::default();
        mem.grow(1024);
        assert_eq!(mem.len(), 1024);

        let region = get_memory_region(&mut mem, 0, 512).expect("memory read failed");
        check_mem!(mem, region, 512);
    }

    #[test]
    fn test_memread_simple2() {
        // simple read in bounds
        let mut mem = Memory::default();
        mem.grow(1024);
        assert_eq!(mem.len(), 1024);

        let region = get_memory_region(&mut mem, 128, 512).expect("memory read failed");
        check_mem!(mem, region, 512);
    }

    #[test]
    fn test_memread_simple3() {
        // simple read in bounds
        let mut mem = Memory::default();
        mem.grow(1024);
        assert_eq!(mem.len(), 1024);

        let region = get_memory_region(&mut mem, 512, 512).expect("memory read failed");
        check_mem!(mem, region, 512);
    }

    #[test]
    fn test_memread_empty() {
        let mut mem = Memory::default();
        mem.grow(1024);
        assert_eq!(mem.len(), 1024);

        let region = get_memory_region(&mut mem, 512, 0).expect("memory read failed");
        assert!(region.is_none());
    }

    #[test]
    fn test_memread_overflow1() {
        // len > mem size
        let mut mem = Memory::default();
        mem.grow(1024);
        assert_eq!(mem.len(), 1024);

        let region = get_memory_region(&mut mem, 0, 2048).expect("memory read failed");
        check_mem!(mem, region, 2048);
    }

    #[test]
    fn test_memread_overflow2() {
        // offset > mem size
        let mut mem = Memory::default();
        mem.grow(1024);
        assert_eq!(mem.len(), 1024);

        let region = get_memory_region(&mut mem, 1056, 1024).expect("memory read failed");
        check_mem!(mem, region, 1024);
    }

    #[test]
    fn test_memread_overflow3() {
        // offset+len > mem size
        let mut mem = Memory::default();
        mem.grow(1024);
        assert_eq!(mem.len(), 1024);

        let region = get_memory_region(&mut mem, 988, 2048).expect("memory read failed");
        check_mem!(mem, region, 2048);
    }

    #[test]
    fn test_memread_overflow_err() {
        let mut mem = Memory::default();

        let result = get_memory_region(&mut mem, u32::MAX - 1, 10);
        assert!(result.is_err());
        assert_eq!(result.err().unwrap().exit_code(), crate::EVM_CONTRACT_ILLEGAL_MEMORY_ACCESS);
    }
}
