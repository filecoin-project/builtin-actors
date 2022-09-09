#!allow[clippy::result-unit-err]

use {
    crate::interpreter::memory::Memory,
    crate::interpreter::{ExecutionState, StatusCode, U256},
    std::num::NonZeroUsize,
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
    if offset > U256::from(u32::MAX) {
        return Err(());
    }

    let new_size = offset.as_usize() + size.get();
    let current_size = mem.len();
    if new_size > current_size {
        grow_memory(mem, new_size)?;
    }

    Ok(MemoryRegion { offset: offset.as_usize(), size })
}

#[inline]
#[allow(clippy::result_unit_err)]
pub fn get_memory_region(
    mem: &mut Memory,
    offset: U256,
    size: U256,
) -> Result<Option<MemoryRegion>, ()> {
    if size == U256::zero() {
        return Ok(None);
    }

    if size > U256::from(u32::MAX) {
        return Err(());
    }

    get_memory_region_u64(mem, offset, NonZeroUsize::new(size.as_usize()).unwrap()).map(Some)
}

#[inline]
pub fn mload(state: &mut ExecutionState) -> Result<(), StatusCode> {
    let index = state.stack.pop()?;

    let region =
        get_memory_region_u64(&mut state.memory, index, NonZeroUsize::new(WORD_SIZE).unwrap())
            .map_err(|_| StatusCode::InvalidMemoryAccess)?;
    let value =
        U256::from_big_endian(&state.memory[region.offset..region.offset + region.size.get()]);

    state.stack.push(value)
}

#[inline]
pub fn mstore(state: &mut ExecutionState) -> Result<(), StatusCode> {
    let (index, value) = state.stack.with::<2,_,_>(|args| {
        Ok((args[1], args[0]))
    })?;

    let region =
        get_memory_region_u64(&mut state.memory, index, NonZeroUsize::new(WORD_SIZE).unwrap())
            .map_err(|_| StatusCode::InvalidMemoryAccess)?;

    let mut bytes = [0u8; WORD_SIZE];
    value.to_big_endian(&mut bytes);
    state.memory[region.offset..region.offset + WORD_SIZE].copy_from_slice(&bytes);

    Ok(())
}

#[inline]
pub fn mstore8(state: &mut ExecutionState) -> Result<(), StatusCode> {
    let (index, value) = state.stack.with::<2,_,_>(|args| {
        Ok((args[1], args[0]))
    })?;

    let region = get_memory_region_u64(&mut state.memory, index, NonZeroUsize::new(1).unwrap())
        .map_err(|_| StatusCode::InvalidMemoryAccess)?;

    let value = (value.low_u32() & 0xff) as u8;

    state.memory[region.offset] = value;

    Ok(())
}

#[inline]
pub fn msize(state: &mut ExecutionState) -> Result<(), StatusCode> {
    state.stack.push(u64::try_from(state.memory.len()).unwrap().into())
}
