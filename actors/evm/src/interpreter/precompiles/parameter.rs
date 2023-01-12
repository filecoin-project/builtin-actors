use std::{borrow::Cow, slice::ChunksExact};

use substrate_bn::{AffineG1, FieldError, Fq, Group, GroupError, G1};

use crate::interpreter::U256;

use super::PrecompileError;

impl From<FieldError> for PrecompileError {
    fn from(src: FieldError) -> Self {
        PrecompileError::EcErr(src.into())
    }
}

impl From<GroupError> for PrecompileError {
    fn from(src: GroupError) -> Self {
        PrecompileError::EcGroupErr(src)
    }
}

/// Pad out to len as needed, but does not slice output to len
pub(super) fn right_pad<'a>(input: impl Into<Cow<'a, [u8]>>, len: usize) -> Cow<'a, [u8]> {
    let mut input: Cow<[u8]> = input.into();
    let input_len = input.len();
    if len > input_len {
        input.to_mut().resize(len, 0);
    }
    input
}

/// ensures top bits are zeroed
pub fn assert_zero_bytes<const S: usize>(src: &[u8]) -> Result<(), PrecompileError> {
    if src[..S] != [0u8; S] {
        Err(PrecompileError::InvalidInput)
    } else {
        Ok(())
    }
}

pub(super) struct Parameter<T>(pub T);

impl<'a> TryFrom<&'a [u8; 64]> for Parameter<G1> {
    type Error = PrecompileError;

    fn try_from(value: &'a [u8; 64]) -> Result<Self, Self::Error> {
        let x = Fq::from_u256(U256::from_big_endian(&value[0..32]).into())?;
        let y = Fq::from_u256(U256::from_big_endian(&value[32..64]).into())?;

        Ok(if x.is_zero() && y.is_zero() {
            Parameter(G1::zero())
        } else {
            Parameter(AffineG1::new(x, y)?.into())
        })
    }
}

impl<'a> From<&'a [u8; 32]> for Parameter<[u8; 32]> {
    fn from(value: &'a [u8; 32]) -> Self {
        Self(*value)
    }
}

impl<'a> TryFrom<&'a [u8; 32]> for Parameter<u32> {
    type Error = PrecompileError;

    fn try_from(value: &'a [u8; 32]) -> Result<Self, Self::Error> {
        assert_zero_bytes::<28>(value)?;
        // Type ensures our remaining len == 4
        Ok(Self(u32::from_be_bytes(value[28..].try_into().unwrap())))
    }
}

impl<'a> TryFrom<&'a [u8; 32]> for Parameter<i32> {
    type Error = PrecompileError;

    fn try_from(value: &'a [u8; 32]) -> Result<Self, Self::Error> {
        assert_zero_bytes::<28>(value)?;
        // Type ensures our remaining len == 4
        Ok(Self(i32::from_be_bytes(value[28..].try_into().unwrap())))
    }
}

impl<'a> TryFrom<&'a [u8; 32]> for Parameter<u8> {
    type Error = PrecompileError;

    fn try_from(value: &'a [u8; 32]) -> Result<Self, Self::Error> {
        assert_zero_bytes::<31>(value)?;
        Ok(Self(value[31]))
    }
}

impl<'a> TryFrom<&'a [u8; 32]> for Parameter<u64> {
    type Error = PrecompileError;

    fn try_from(value: &'a [u8; 32]) -> Result<Self, Self::Error> {
        assert_zero_bytes::<24>(value)?;
        // Type ensures our remaining len == 8
        Ok(Self(u64::from_be_bytes(value[24..].try_into().unwrap())))
    }
}

impl<'a> TryFrom<&'a [u8; 32]> for Parameter<i64> {
    type Error = PrecompileError;

    fn try_from(value: &'a [u8; 32]) -> Result<Self, Self::Error> {
        assert_zero_bytes::<24>(value)?;
        // Type ensures our remaining len == 8
        Ok(Self(i64::from_be_bytes(value[24..].try_into().unwrap())))
    }
}

impl<'a> From<&'a [u8; 32]> for Parameter<U256> {
    fn from(value: &'a [u8; 32]) -> Self {
        Self(U256::from_big_endian(value))
    }
}

pub(super) type U256Reader<'a> = PaddedChunks<'a, u8, 32>;

// will be nicer with https://github.com/rust-lang/rust/issues/74985
/// Wrapper around `ChunksExact` that pads instead of overflowing.
/// Also provides a nice API interface for reading Parameters from input
pub(super) struct PaddedChunks<'a, T: Sized + Copy, const CHUNK_SIZE: usize> {
    slice: &'a [T],
    chunks: ChunksExact<'a, T>,
    exhausted: bool,
}

impl<'a, T: Sized + Copy, const CHUNK_SIZE: usize> PaddedChunks<'a, T, CHUNK_SIZE> {
    pub(super) fn new(slice: &'a [T]) -> Self {
        Self { slice, chunks: slice.chunks_exact(CHUNK_SIZE), exhausted: false }
    }

    pub fn next(&mut self) -> Option<&[T; CHUNK_SIZE]> {
        self.chunks.next().map(|s| s.try_into().unwrap())
    }

    pub fn next_padded(&mut self) -> [T; CHUNK_SIZE]
    where
        T: Default,
    {
        if self.chunks.len() > 0 {
            self.next().copied().unwrap_or([T::default(); CHUNK_SIZE])
        } else if self.exhausted() {
            [T::default(); CHUNK_SIZE]
        } else {
            self.exhausted = true;
            let mut buf = [T::default(); CHUNK_SIZE];
            let remainder = self.chunks.remainder();
            buf[..remainder.len()].copy_from_slice(remainder);
            buf
        }
    }

    pub fn exhausted(&self) -> bool {
        self.exhausted
    }

    pub fn remaining_len(&self) -> usize {
        if self.exhausted {
            0
        } else {
            self.chunks.len() * CHUNK_SIZE + self.chunks.remainder().len()
        }
    }

    pub fn chunks_read(&self) -> usize {
        let total_chunks = self.slice.len() / CHUNK_SIZE;
        let unread_chunks = self.chunks.len();
        total_chunks - unread_chunks
    }

    // remaining unpadded slice of unread items
    pub fn remaining_slice(&self) -> &[T] {
        let start = self.slice.len() - self.remaining_len();
        &self.slice[start..]
    }

    // // tries to read an unpadded and exact (aligned) parameter
    #[allow(unused)]
    pub fn next_param<V>(&mut self) -> Result<V, PrecompileError>
    where
        Parameter<V>: for<'from> TryFrom<&'from [T; CHUNK_SIZE], Error = PrecompileError>,
    {
        Parameter::<V>::try_from(self.next().ok_or(PrecompileError::IncorrectInputSize)?)
            .map(|a| a.0)
    }

    // tries to read a parameter with padding
    pub fn next_param_padded<V>(&mut self) -> Result<V, PrecompileError>
    where
        T: Default,
        Parameter<V>: for<'from> TryFrom<&'from [T; CHUNK_SIZE], Error = PrecompileError>,
    {
        Parameter::<V>::try_from(&self.next_padded()).map(|a| a.0)
    }

    #[allow(unused)]
    pub fn next_into_param_padded<V>(&mut self) -> V
    where
        T: Default,
        Parameter<V>: for<'from> From<&'from [T; CHUNK_SIZE]>,
    {
        Parameter::<V>::from(&self.next_padded()).0
    }

    // read a parameter with padding
    pub fn next_into_param<V>(&mut self) -> Result<V, PrecompileError>
    where
        T: Default,
        Parameter<V>: for<'from> From<&'from [T; CHUNK_SIZE]>,
    {
        self.next().map(|p| Parameter::<V>::from(p).0).ok_or(PrecompileError::IncorrectInputSize)
    }
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn test_assert_zero_bytes() {
        let mut bytes = [0u8; 32];
        assert_zero_bytes::<32>(&bytes).unwrap();
        bytes[31] = 1;
        assert_zero_bytes::<31>(&bytes).unwrap();
        assert_zero_bytes::<32>(&bytes).expect_err("expected error from nonzero byte");
    }
}
