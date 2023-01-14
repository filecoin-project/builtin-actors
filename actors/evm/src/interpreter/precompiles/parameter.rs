use std::borrow::Cow;

use fvm_shared::bigint::BigUint;
use substrate_bn::{AffineG1, FieldError, Fq, Fr, Group, GroupError, G1};

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

pub(super) trait Parameter: Sized {
    fn read(reader: &mut ParameterReader) -> Result<Self, PrecompileError>;
}

impl Parameter for G1 {
    fn read(reader: &mut ParameterReader) -> Result<Self, PrecompileError> {
        let x: Fq = reader.read_param()?;
        let y: Fq = reader.read_param()?;

        Ok(if x.is_zero() && y.is_zero() { G1::zero() } else { AffineG1::new(x, y)?.into() })
    }
}

impl Parameter for Fq {
    fn read(reader: &mut ParameterReader) -> Result<Self, PrecompileError> {
        Ok(Fq::from_slice(&reader.read_fixed::<32>())?)
    }
}

impl Parameter for Fr {
    fn read(reader: &mut ParameterReader) -> Result<Self, PrecompileError> {
        Ok(Fr::from_slice(&reader.read_fixed::<32>())?)
    }
}

impl Parameter for U256 {
    fn read(reader: &mut ParameterReader) -> Result<Self, PrecompileError> {
        Ok(U256::from(reader.read_fixed::<32>()))
    }
}

impl Parameter for [u8; 32] {
    fn read(reader: &mut ParameterReader) -> Result<Self, PrecompileError> {
        Ok(reader.read_fixed())
    }
}

impl Parameter for u8 {
    fn read(reader: &mut ParameterReader) -> Result<Self, PrecompileError> {
        reader.drop_zeros::<31>()?;
        Ok(reader.read_byte())
    }
}

macro_rules! impl_param_int {
    ($($t:ty)*) => {
        $(
            impl Parameter for $t {
                fn read(reader: &mut ParameterReader) -> Result<Self, PrecompileError> {
                    const ZEROS: usize = 32 - ((<$t>::BITS as usize) / 8);

                    reader.drop_zeros::<ZEROS>()?;
                    // Type ensures our remaining len
                    Ok(<$t>::from_be_bytes(reader.read_fixed()))
                }
            }
        )*
    };
}

impl_param_int!(u16 i16 u32 i32 u64 i64);

/// Provides a nice API interface for reading Parameters from input. This API treats the input as if
/// it is followed by infinite zeros.
pub(super) struct ParameterReader<'a> {
    full: &'a [u8],
    slice: &'a [u8],
}

impl<'a> ParameterReader<'a> {
    pub(super) fn new(slice: &'a [u8]) -> Self {
        ParameterReader { full: slice, slice }
    }

    /// Seek to an offset from the beginning of the input.
    pub fn seek(&mut self, offset: usize) {
        if offset > self.full.len() {
            self.slice = &[];
        } else {
            self.slice = &self.full[offset..];
        }
    }

    /// Drop a fixed number of bytes, and return an error if said bytes are not zeros.
    pub fn drop_zeros<const S: usize>(&mut self) -> Result<(), PrecompileError> {
        let split = S.min(self.slice.len());
        let (a, b) = self.slice.split_at(split);
        self.slice = b;
        if a.iter().all(|&i| i == 0) {
            Ok(())
        } else {
            Err(PrecompileError::InvalidInput)
        }
    }

    /// Read a single byte, or 0 if there's no remaining input.
    ///
    /// NOTE: This won't read 32 bytes, it'll just read a _single_ byte.
    pub fn read_byte(&mut self) -> u8 {
        if let Some((&first, rest)) = self.slice.split_first() {
            self.slice = rest;
            first
        } else {
            0
        }
    }

    /// Read a fixed number of bytes from the input, zero-padding as necessary.
    ///
    /// NOTE: this won't read in 32byte chunks, it'll read the specified number of bytes exactly.
    pub fn read_fixed<const S: usize>(&mut self) -> [u8; S] {
        let mut out = [0u8; S];
        let split = S.min(self.slice.len());
        let (a, b) = self.slice.split_at(split);
        self.slice = b;
        out[..split].copy_from_slice(a);
        out
    }

    /// Read input and pad up to `len`.
    pub fn read_padded(&mut self, len: usize) -> Cow<'a, [u8]> {
        if len <= self.slice.len() {
            let (a, b) = self.slice.split_at(len);
            self.slice = b;
            Cow::Borrowed(a)
        } else {
            let mut buf = Vec::with_capacity(len);
            buf.extend_from_slice(self.slice);
            buf.resize(len, 0);
            self.slice = &[];
            Cow::Owned(buf)
        }
    }

    /// Read a bigint from the input, and pad up to `len`.
    pub fn read_biguint(&mut self, len: usize) -> BigUint {
        // We read the bigint in two steps:
        // 1. We read any bytes that are actually present in the input.
        // 2. Then we pad by _shifting_ the integer by the number of missing bits.
        let split = len.min(self.slice.len());
        let (a, b) = self.slice.split_at(split);
        self.slice = b;

        // Start with the existing bytes.
        let mut int = BigUint::from_bytes_be(a);
        // Then shift, if necessary.
        if split < len {
            int <<= ((len - split) as u32) * u8::BITS;
        }
        int
    }

    /// Read a single parameter from the input. The parameter's type decides how much input it needs
    /// to read.
    ///
    /// Most parameters will read in 32 byte chunks, but that's up to the parameter's
    /// implementation.
    pub fn read_param<V>(&mut self) -> Result<V, PrecompileError>
    where
        V: Parameter,
    {
        Parameter::read(self)
    }
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn test_read_fixed() {
        let mut reader = ParameterReader::new(&[1, 2, 3]);
        assert_eq!(reader.read_fixed::<2>(), [1, 2]);
        assert_eq!(reader.read_fixed::<0>(), []);
        assert_eq!(reader.read_fixed::<5>(), [3u8, 0, 0, 0, 0]);
        assert_eq!(reader.read_fixed::<3>(), [0, 0, 0]);
    }

    #[test]
    fn test_right_pad() {
        let mut reader = ParameterReader::new(&[1, 2, 3]);
        assert_eq!(reader.read_padded(2), &[1, 2][..]);
        assert_eq!(reader.read_padded(2), &[3, 0][..]);
        assert_eq!(reader.read_padded(2), &[0, 0][..]);
    }

    #[test]
    fn test_int() {
        let mut data = vec![0u8; 37];
        data[31] = 1;
        let mut reader = ParameterReader::new(&data);
        assert_eq!(reader.read_param::<u64>().unwrap(), 1);
        assert_eq!(reader.read_param::<u64>().unwrap(), 0);

        // Expect this to overflow now.
        data[0] = 1;
        let mut reader = ParameterReader::new(&data);
        assert!(matches!(reader.read_param::<u64>().unwrap_err(), PrecompileError::InvalidInput));
    }

    #[test]
    fn test_big_int() {
        let mut reader = ParameterReader::new(&[1, 2]);
        assert_eq!(reader.read_biguint(1), 1u64.into());
        assert_eq!(reader.read_biguint(3), 0x02_00_00u64.into());
        assert_eq!(reader.read_biguint(5), 0u32.into());
    }

    #[test]
    fn test_byte() {
        let mut reader = ParameterReader::new(&[1, 2]);
        assert_eq!(reader.read_byte(), 1u8);
        assert_eq!(reader.read_byte(), 2u8);
        assert_eq!(reader.read_byte(), 0u8);
        assert_eq!(reader.read_byte(), 0u8);
    }
}
