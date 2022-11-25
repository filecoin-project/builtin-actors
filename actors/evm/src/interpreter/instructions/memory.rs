#!allow[clippy::result-unit-err]

use {
    crate::interpreter::memory::Memory,
    crate::interpreter::{ExecutionState, System, StatusCode, U256},
    std::num::NonZeroUsize,
    fil_actors_runtime::runtime::Runtime,
};

/// The size of the EVM 256-bit word in bytes.
const WORD_SIZE: usize = 32;

#[derive(Debug)]
pub struct MemoryRegion {
    pub offset: usize,
    pub size: NonZeroUsize,
}

/// Returns number of words what would fit to provided number of bytes,
/// i.e. it rounds up the number bytes to number of words.
#[inline]
pub fn num_words(size_in_bytes: usize) -> usize {
    (size_in_bytes + (WORD_SIZE - 1)) / WORD_SIZE
}

#[inline]
fn grow_memory(mem: &mut Memory, new_size: usize) -> Result<(), ()> {
    let new_words = num_words(new_size);
    mem.grow((new_words * WORD_SIZE) as usize);
    Ok(())
}

#[inline]
fn get_memory_region_u64(
    mem: &mut Memory,
    offset: U256,
    size: NonZeroUsize,
) -> Result<MemoryRegion, ()> {
    if offset.bits() >= 32 {
        return Err(());
    }

    let offset_usize = offset.as_usize();
    let new_size = offset_usize + size.get();
    let current_size = mem.len();
    if new_size > current_size {
        grow_memory(mem, new_size)?;
    }

    Ok(MemoryRegion { offset: offset_usize, size })
}

#[inline]
#[allow(clippy::result_unit_err)]
pub fn get_memory_region(
    mem: &mut Memory,
    offset: U256,
    size: U256,
) -> Result<Option<MemoryRegion>, ()> {
    if size.is_zero() {
        return Ok(None);
    }

    if size.bits() >= 32 {
        return Err(());
    }

    get_memory_region_u64(mem, offset, NonZeroUsize::new(size.as_usize()).unwrap()).map(Some)
}

pub fn copy_to_memory(
    memory: &mut Memory,
    dest_offset: U256,
    dest_size: U256,
    data_offset: U256,
    data: &[u8],
    zero_fill: bool,
) -> Result<(), StatusCode> {
    let region = get_memory_region(memory, dest_offset, dest_size)
        .map_err(|_| StatusCode::InvalidMemoryAccess)?;

    #[inline(always)]
    fn min(a: U256, b: usize) -> usize {
        if a < (b as u64) {
            a.as_usize()
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
pub fn mload(state: &mut ExecutionState, _system: &System<impl Runtime>, index: U256) -> Result<U256, StatusCode> {
    let region =
        get_memory_region_u64(&mut state.memory, index, NonZeroUsize::new(WORD_SIZE).unwrap())
            .map_err(|_| StatusCode::InvalidMemoryAccess)?;
    let value =
        U256::from_big_endian(&state.memory[region.offset..region.offset + region.size.get()]);

    Ok(value)
}

#[inline]
pub fn mstore(state: &mut ExecutionState, _system: &System<impl Runtime>, index: U256, value: U256) -> Result<(), StatusCode> {
    let region =
        get_memory_region_u64(&mut state.memory, index, NonZeroUsize::new(WORD_SIZE).unwrap())
            .map_err(|_| StatusCode::InvalidMemoryAccess)?;

    let mut bytes = [0u8; WORD_SIZE];
    value.to_big_endian(&mut bytes);
    state.memory[region.offset..region.offset + WORD_SIZE].copy_from_slice(&bytes);

    Ok(())
}

#[inline]
pub fn mstore8(state: &mut ExecutionState, _system: &System<impl Runtime>, index: U256, value: U256) -> Result<(), StatusCode> {
    let region = get_memory_region_u64(&mut state.memory, index, NonZeroUsize::new(1).unwrap())
        .map_err(|_| StatusCode::InvalidMemoryAccess)?;

    let value = (value.low_u32() & 0xff) as u8;
    state.memory[region.offset] = value;

    Ok(())
}

#[inline]
pub fn msize(state: &mut ExecutionState,  _system: &System<impl Runtime>) -> Result<U256, StatusCode> {
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
        assert_eq!(result, Err(StatusCode::InvalidMemoryAccess));
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
