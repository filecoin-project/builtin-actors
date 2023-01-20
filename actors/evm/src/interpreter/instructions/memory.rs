#!allow[clippy::result-unit-err]

use fil_actors_runtime::{ActorError, AsActorError};

use crate::EVM_CONTRACT_ILLEGAL_MEMORY_ACCESS;

use {
    crate::interpreter::memory::Memory,
    crate::interpreter::{ExecutionState, System, U256},
    fil_actors_runtime::runtime::Runtime,
    std::num::NonZeroUsize,
};

/// The size of the EVM 256-bit word in bytes.
const WORD_SIZE: usize = 32;

#[derive(Debug)]
pub struct MemoryRegion {
    pub offset: usize,
    pub size: NonZeroUsize,
}

#[inline]
fn grow_memory(mem: &mut Memory, mut new_size: usize) {
    // Align to the next u256.
    // Guaranteed to not overflow.
    let alignment = new_size % WORD_SIZE;
    if alignment > 0 {
        new_size += WORD_SIZE - alignment;
    }
    mem.grow(new_size);
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

    let current_size = mem.len();
    if new_size as usize > current_size {
        grow_memory(mem, new_size as usize);
    }

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
    let region = get_memory_region(&mut state.memory, index, WORD_SIZE)?.expect("empty region");
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
    let region = get_memory_region(&mut state.memory, index, WORD_SIZE)?.expect("empty region");

    let mut bytes = [0u8; WORD_SIZE];
    value.to_big_endian(&mut bytes);
    state.memory[region.offset..region.offset + WORD_SIZE].copy_from_slice(&bytes);

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
    use crate::interpreter::memory::Memory;

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
}
