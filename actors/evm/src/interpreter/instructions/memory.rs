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
    if offset.bits() >= 32 {
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
    data_offset: U256,
    size: U256,
    data: &[u8],
) -> Result<(), StatusCode> {
    // TODO this limits addressable output to 2G (31 bits full),
    //      but it is still probably too much and we should consistently limit further.
    //      See also https://github.com/filecoin-project/ref-fvm/issues/851
    if size.bits() >= 32 {
        return Err(StatusCode::InvalidMemoryAccess);
    }
    let output_usize = size.as_usize();

    if output_usize > 0 {
        let output_region = get_memory_region(memory, dest_offset, size)
            .map_err(|_| StatusCode::InvalidMemoryAccess)?;
        let output_data = output_region
            .map(|MemoryRegion { offset, size }| &mut memory[offset..][..size.get()])
            .ok_or(StatusCode::InvalidMemoryAccess)?;

        // Limit the size if we're copying less than the data length.
        let mut copy_len = data.len();
        if output_usize < copy_len {
            copy_len = output_usize;
        }

        output_data
            .get_mut(..copy_len)
            .ok_or(StatusCode::InvalidMemoryAccess)?
            .copy_from_slice(&data[data_offset.as_usize()..][..copy_len]);
    }

    Ok(())
}

#[inline]
pub fn mload(state: &mut ExecutionState) -> Result<(), StatusCode> {
    let index = state.stack.pop();

    let region =
        get_memory_region_u64(&mut state.memory, index, NonZeroUsize::new(WORD_SIZE).unwrap())
            .map_err(|_| StatusCode::InvalidMemoryAccess)?;
    let value =
        U256::from_big_endian(&state.memory[region.offset..region.offset + region.size.get()]);

    state.stack.push(value);

    Ok(())
}

#[inline]
pub fn mstore(state: &mut ExecutionState) -> Result<(), StatusCode> {
    let index = state.stack.pop();
    let value = state.stack.pop();

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
    let index = state.stack.pop();
    let value = state.stack.pop();

    let region = get_memory_region_u64(&mut state.memory, index, NonZeroUsize::new(1).unwrap())
        .map_err(|_| StatusCode::InvalidMemoryAccess)?;

    let value = (value.low_u32() & 0xff) as u8;

    state.memory[region.offset] = value;

    Ok(())
}

#[inline]
pub fn msize(state: &mut ExecutionState) {
    state.stack.push(u64::try_from(state.memory.len()).unwrap().into());
}
