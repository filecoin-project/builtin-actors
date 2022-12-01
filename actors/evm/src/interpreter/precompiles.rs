use std::{borrow::Cow, convert::TryInto, marker::PhantomData, slice::ChunksExact};

use super::{StatusCode, System, U256};

use fil_actors_runtime::runtime::Runtime;
use fvm_ipld_encoding::RawBytes;
use fvm_shared::{
    address::Address,
    bigint::BigUint,
    crypto::{
        hash::SupportedHashes,
        signature::{SECP_SIG_LEN, SECP_SIG_MESSAGE_HASH_SIZE},
    },
    econ::TokenAmount,
};
use num_traits::FromPrimitive;
use num_traits::{One, Zero};
use substrate_bn::{pairing_batch, AffineG1, AffineG2, Fq, Fq2, Fr, Group, Gt, G1, G2};
use uint::byteorder::{ByteOrder, LE};

pub use substrate_bn::{CurveError, FieldError, GroupError};

lazy_static::lazy_static! {
    pub(crate) static ref SECP256K1: BigUint = BigUint::from_bytes_be(&hex_literal::hex!("fffffffffffffffffffffffffffffffebaaedce6af48a03bbfd25e8cd0364141"));
}

/// ensures top bits are zeroed
pub fn assert_zero_bytes<const S: usize>(src: &[u8]) -> Result<(), PrecompileError> {
    if src[..S] != [0u8; S] {
        Err(PrecompileError::InvalidInput)
    } else {
        Ok(())
    }
}

struct Parameter<T>(pub T);

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

type U256Reader<'a> = PaddedChunks<'a, u8, 32>;

// will be nicer with https://github.com/rust-lang/rust/issues/74985
/// Wrapper around `ChunksExact` that pads instead of overflowing.
/// Also provides a nice API interface for reading Parameters from input
struct PaddedChunks<'a, T: Sized + Copy, const CHUNK_SIZE: usize> {
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

#[derive(Debug)]
pub enum PrecompileError {
    EcErr(CurveError),
    EcGroupErr(GroupError),
    InvalidInput, // TODO merge with below?
    IncorrectInputSize,
    OutOfGas,
    CallActorError(StatusCode),
}

impl From<PrecompileError> for StatusCode {
    fn from(src: PrecompileError) -> Self {
        match src {
            PrecompileError::CallActorError(e) => e,
            _ => StatusCode::PrecompileFailure,
        }
    }
}

impl From<CurveError> for PrecompileError {
    fn from(src: CurveError) -> Self {
        PrecompileError::EcErr(src)
    }
}

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

#[derive(Debug, PartialEq, Eq, Default)]
pub struct PrecompileContext {
    pub is_static: bool,
    pub gas: U256,
    pub value: U256,
}

// really I'd want to have context as a type parameter, but since the table we generate must have the same types (or dyn) its messy
type PrecompileFn<RT> = unsafe fn(*mut System<RT>, &[u8], PrecompileContext) -> PrecompileResult;
pub type PrecompileResult = Result<Vec<u8>, PrecompileError>; // TODO i dont like vec

/// Generates a list of precompile smart contracts, index + 1 is the address (another option is to make an enum)
const fn gen_precompiles<RT: Runtime>() -> [PrecompileFn<RT>; 14] {
    macro_rules! precompiles {
        ($($precompile:ident,)*) => {
            mod trampolines {
                use fil_actors_runtime::runtime::Runtime;
                use crate::System;
                use super::{PrecompileContext, PrecompileResult};
                $(
                    #[inline(always)]
                    pub unsafe fn $precompile<RT: Runtime>(s: *mut System<RT>, inp: &[u8], ctx: PrecompileContext) -> PrecompileResult {
                        super::$precompile(&mut *s, inp, ctx)
                    }
                )*
            }
            [
                $(trampolines::$precompile,)*
            ]
        }
    }

    precompiles! {
        ec_recover, // ecrecover 0x01
        sha256,     // SHA2-256 0x02
        ripemd160,  // ripemd160 0x03
        identity,   // identity 0x04
        modexp,     // modexp 0x05
        ec_add,     // ecAdd 0x06
        ec_mul,     // ecMul 0x07
        ec_pairing, // ecPairing 0x08
        blake2f,    // blake2f 0x09
        // FIL precompiles
        resolve_address,    // lookup_address 0x0a
        lookup_address,     // resolve_address 0x0b
        get_actor_code_cid, // get code cid 0x0c
        get_randomness,     // rand 0x0d
        call_actor,         // call_actor 0x0e
    }
}

pub struct Precompiles<RT>(PhantomData<RT>);

impl<RT: Runtime> Precompiles<RT> {
    const PRECOMPILES: [PrecompileFn<RT>; 14] = gen_precompiles();
    const MAX_PRECOMPILE: U256 = {
        let mut limbs = [0u64; 4];
        limbs[0] = Self::PRECOMPILES.len() as u64;
        U256(limbs)
    };

    // Precompile Context will be flattened to None if not calling the call_actor precompile
    pub fn call_precompile(
        system: &mut System<RT>,
        precompile_addr: U256,
        input: &[u8],
        context: PrecompileContext,
    ) -> PrecompileResult {
        unsafe { Self::PRECOMPILES[precompile_addr.0[0] as usize - 1](system, input, context) }
    }

    #[inline]
    pub fn is_precompile(addr: &U256) -> bool {
        !addr.is_zero() && addr <= &Self::MAX_PRECOMPILE
    }
}

// It is uncomfortable how much Eth pads everything...
fn read_right_pad<'a>(input: impl Into<Cow<'a, [u8]>>, len: usize) -> Cow<'a, [u8]> {
    let mut input: Cow<[u8]> = input.into();
    let input_len = input.len();
    if len > input_len {
        input.to_mut().resize(len, 0);
    }
    input
}

// --- Precompiles ---

/// Read right padded BE encoded low u64 ID address from a u256 word
/// returns encoded CID or an empty array if actor not found
fn get_actor_code_cid<RT: Runtime>(
    system: &mut System<RT>,
    input: &[u8],
    _: PrecompileContext,
) -> PrecompileResult {
    let id_bytes: [u8; 32] = read_right_pad(input, 32).as_ref().try_into().unwrap();
    let id = Parameter::<u64>::try_from(&id_bytes)?.0;
    Ok(system.rt.get_actor_code_cid(&id).unwrap_or_default().to_bytes())
}

/// Params:
///
/// | Param            | Value                     |
/// |------------------|---------------------------|
/// | randomness_type  | U256 - low i32: `Chain`(0) OR `Beacon`(1) |
/// | personalization  | U256 - low i64             |
/// | randomness_epoch | U256 - low i64             |
/// | entropy_length   | U256 - low u32             |
/// | entropy          | input\[32..] (right padded)|
///
/// any bytes in between values are ignored
///
/// Returns empty array if invalid randomness type
/// Errors if unable to fetch randomness
fn get_randomness<RT: Runtime>(
    system: &mut System<RT>,
    input: &[u8],
    _: PrecompileContext,
) -> PrecompileResult {
    let mut input_params = U256Reader::new(input);

    #[derive(num_derive::FromPrimitive)]
    #[repr(i32)]
    enum RandomnessType {
        Chain = 0,
        Beacon = 1,
    }

    let randomness_type = RandomnessType::from_i32(input_params.next_param_padded::<i32>()?);
    let personalization = input_params.next_param_padded::<i64>()?;
    let rand_epoch = input_params.next_param_padded::<i64>()?;
    let entropy_len = input_params.next_param_padded::<u32>()?;

    debug_assert_eq!(input_params.chunks_read(), 4);

    let entropy = read_right_pad(input_params.remaining_slice(), entropy_len as usize);

    let randomness = match randomness_type {
        Some(RandomnessType::Chain) => system
            .rt
            .user_get_randomness_from_chain(personalization, rand_epoch, &entropy)
            .map(|a| a.to_vec()),
        Some(RandomnessType::Beacon) => system
            .rt
            .user_get_randomness_from_beacon(personalization, rand_epoch, &entropy)
            .map(|a| a.to_vec()),
        None => Ok(Vec::new()),
    };

    randomness.map_err(|_| PrecompileError::InvalidInput)
}

/// Read BE encoded low u64 ID address from a u256 word
/// Looks up and returns the other address (encoded f2 or f4 addresses) of an ID address, returning empty array if not found
fn lookup_address<RT: Runtime>(
    system: &mut System<RT>,
    input: &[u8],
    _: PrecompileContext,
) -> PrecompileResult {
    let mut id_bytes = U256Reader::new(input);
    let id = id_bytes.next_param_padded::<u64>()?;

    let address = system.rt.lookup_address(id);
    let ab = match address {
        Some(a) => a.to_bytes(),
        None => Vec::new(),
    };
    Ok(ab)
}

/// Reads a FIL encoded address
/// Resolves a FIL encoded address into an ID address
/// returns BE encoded u64 or empty array if nothing found
fn resolve_address<RT: Runtime>(
    system: &mut System<RT>,
    input: &[u8],
    _: PrecompileContext,
) -> PrecompileResult {
    let mut input_params = U256Reader::new(input);

    let len = input_params.next_param_padded::<u32>()? as usize;
    let addr = match Address::from_bytes(&read_right_pad(input_params.remaining_slice(), len)) {
        Ok(o) => o,
        Err(_) => return Ok(Vec::new()),
    };
    Ok(system.rt.resolve_address(&addr).map(|a| a.to_be_bytes().to_vec()).unwrap_or_default())
}

/// Errors:
///    TODO should just give 0s?
/// - `IncorrectInputSize` if offset is larger than total input length
/// - `InvalidInput` if supplied address bytes isnt a filecoin address
///
/// Returns:
///
/// `[int256 exit_code, uint codec, uint offset, uint size, []bytes <actor return value>]`
///
/// for exit_code:
/// - negative values are system errors
/// - positive are user errors (from the called actor)
/// - 0 is success
pub fn call_actor<RT: Runtime>(
    system: &mut System<RT>,
    input: &[u8],
    ctx: PrecompileContext,
) -> PrecompileResult {
    // ----- Input Parameters -------

    let mut input_params = U256Reader::new(input);

    let method: u64 = input_params.next_param_padded()?;
    let codec: u64 = input_params.next_param_padded()?;
    // TODO only CBOR for now
    if codec != fvm_ipld_encoding::DAG_CBOR {
        return Err(PrecompileError::InvalidInput);
    }

    let address_size = input_params.next_param_padded::<u32>()? as usize;
    let send_data_size = input_params.next_param_padded::<u32>()? as usize;

    // ------ Begin Call -------

    let result = {
        // REMOVEME: closes https://github.com/filecoin-project/ref-fvm/issues/1018

        let start = input_params.remaining_slice();
        let bytes = read_right_pad(start, send_data_size + address_size);

        let input_data = &bytes[..send_data_size];
        let address = &bytes[send_data_size..send_data_size + address_size];
        let address = Address::from_bytes(address).map_err(|_| PrecompileError::InvalidInput)?;

        system.send_with_gas(
            &address,
            method,
            RawBytes::from(input_data.to_vec()),
            TokenAmount::from(&ctx.value),
            if !ctx.gas.is_zero() { Some(ctx.gas.to_u64_saturating()) } else { None },
            ctx.is_static,
        )
    };

    // ------ Build Output -------

    let output = {
        // negative values are syscall errors
        // positive values are user/actor errors
        // success is 0
        let (exit_code, data) = match result {
            Err(ae) => {
                // TODO handle revert
                // TODO https://github.com/filecoin-project/ref-fvm/issues/1020
                // put error number from call into revert
                let exit_code = U256::from(ae.exit_code().value());

                // no return only exit code
                (exit_code, RawBytes::default())
            }
            Ok(ret) => (U256::zero(), ret),
        };

        const NUM_OUTPUT_PARAMS: u32 = 4;

        // codec of return data
        // TODO hardcoded to CBOR for now
        let codec = U256::from(fvm_ipld_encoding::DAG_CBOR);
        let offset = U256::from(NUM_OUTPUT_PARAMS * 32);
        let size = U256::from(data.len() as u32);

        let mut output = Vec::with_capacity(NUM_OUTPUT_PARAMS as usize * 32 + data.len());
        output.extend_from_slice(&exit_code.to_bytes());
        output.extend_from_slice(&codec.to_bytes());
        output.extend_from_slice(&offset.to_bytes());
        output.extend_from_slice(&size.to_bytes());
        // NOTE:
        // we dont pad out to 32 bytes here, the idea being that users will already be in the "everythig is bytes" mode
        // and will want re-pack align and whatever else by themselves
        output.extend_from_slice(&data);
        output
    };

    Ok(output)
}

// ---------------- Normal EVM Precompiles ------------------

/// recover a secp256k1 pubkey from a hash, recovery byte, and a signature
fn ec_recover<RT: Runtime>(
    system: &mut System<RT>,
    input: &[u8],
    _: PrecompileContext,
) -> PrecompileResult {
    let mut input_params = U256Reader::new(input);

    let hash: [u8; SECP_SIG_MESSAGE_HASH_SIZE] = input_params.next_padded();
    let recovery_byte = input_params.next_param_padded::<u8>();
    let r = input_params.next_padded();
    let s = input_params.next_padded();
    let big_r = BigUint::from_bytes_be(&r);
    let big_s = BigUint::from_bytes_be(&s);

    // recovery byte is a single byte value but is represented with 32 bytes, sad
    let v = match recovery_byte {
        Ok(re) => {
            if matches!(re, 27 | 28) {
                re - 27
            } else {
                return Ok(Vec::new());
            }
        }
        _ => return Ok(Vec::new()),
    };

    let valid = if big_r <= BigUint::one() || big_s <= BigUint::one() {
        false
    } else {
        big_r <= *SECP256K1 && big_s <= *SECP256K1 && (v == 0 || v == 1)
    };

    if !valid {
        return Ok(Vec::new());
    }

    let mut sig: [u8; SECP_SIG_LEN] = [0u8; 65];
    sig[..32].copy_from_slice(&r);
    sig[32..64].copy_from_slice(&s);
    sig[64] = v;

    let pubkey = if let Ok(key) = system.rt.recover_secp_public_key(&hash, &sig) {
        key
    } else {
        return Ok(Vec::new());
    };

    let mut address = system.rt.hash(SupportedHashes::Keccak256, &pubkey[1..]);
    address[..12].copy_from_slice(&[0u8; 12]);

    Ok(address)
}

/// hash with sha2-256
fn sha256<RT: Runtime>(
    system: &mut System<RT>,
    input: &[u8],
    _: PrecompileContext,
) -> PrecompileResult {
    Ok(system.rt.hash(SupportedHashes::Sha2_256, input))
}

/// hash with ripemd160
fn ripemd160<RT: Runtime>(
    system: &mut System<RT>,
    input: &[u8],
    _: PrecompileContext,
) -> PrecompileResult {
    Ok(system.rt.hash(SupportedHashes::Ripemd160, input))
}

/// data copy
fn identity<RT: Runtime>(
    _: &mut System<RT>,
    input: &[u8],
    _: PrecompileContext,
) -> PrecompileResult {
    Ok(Vec::from(input))
}

// https://eips.ethereum.org/EIPS/eip-198
/// modulus exponent a number
fn modexp<RT: Runtime>(_: &mut System<RT>, input: &[u8], _: PrecompileContext) -> PrecompileResult {
    let input = read_right_pad(input, 96);

    // Follows go-ethereum by truncating bits to u64, ignoring other all other values in the first 24 bytes.
    // Since we don't have any complexity functions or specific gas measurements of modexp in FEVM,
    // we let values be whatever and have FEVM gas accounting be the one responsible for keeping things within reasonable limits.
    // We _also_ will default with 0 (though this is already done with right padding above) since that is expected to be fine.
    // Eth really relies heavily on gas checking being correct and safe for client nodes...
    fn read_bigint_len(input: &[u8], start: usize) -> Result<usize, PrecompileError> {
        let digits = BigUint::from_bytes_be(&input[start..start + 32]);
        let mut digits = digits.iter_u64_digits();
        // truncate to 64 bits
        digits
            .next()
            .or(Some(0))
            // wont ever error here, just a type conversion
            .ok_or(PrecompileError::OutOfGas)
            .and_then(|d| u32::try_from(d).map_err(|_| PrecompileError::OutOfGas))
            .map(|d| d as usize)
    }

    let base_len = read_bigint_len(&input, 0)?;
    let exponent_len = read_bigint_len(&input, 32)?;
    let mod_len = read_bigint_len(&input, 64)?;

    if base_len == 0 && mod_len == 0 {
        return Ok(Vec::new());
    }
    let input = if input.len() > 96 { &input[96..] } else { &[] };
    let input = read_right_pad(input, base_len + exponent_len + mod_len);

    let base = BigUint::from_bytes_be(&input[0..base_len]);
    let exponent = BigUint::from_bytes_be(&input[base_len..exponent_len + base_len]);
    let modulus =
        BigUint::from_bytes_be(&input[base_len + exponent_len..mod_len + base_len + exponent_len]);

    if modulus.is_zero() || modulus.is_one() {
        // mod 0 is undefined: 0, base mod 1 is always 0
        return Ok(vec![0; mod_len]);
    }

    let mut output = base.modpow(&exponent, &modulus).to_bytes_be();

    if output.len() < mod_len {
        let mut ret = Vec::with_capacity(mod_len);
        ret.resize(mod_len - output.len(), 0); // left padding
        ret.extend_from_slice(&output);
        output = ret;
    }

    Ok(output)
}

fn curve_to_vec(curve: G1) -> Vec<u8> {
    AffineG1::from_jacobian(curve)
        .map(|product| {
            let mut output = vec![0; 64];
            product.x().to_big_endian(&mut output[0..32]).unwrap();
            product.y().to_big_endian(&mut output[32..64]).unwrap();
            output
        })
        .unwrap_or_else(|| vec![0; 64])
}

/// add 2 points together on an elliptic curve
fn ec_add<RT: Runtime>(_: &mut System<RT>, input: &[u8], _: PrecompileContext) -> PrecompileResult {
    let mut input_params: PaddedChunks<u8, 64> = PaddedChunks::new(input);
    let point1 = input_params.next_param_padded()?;
    let point2 = input_params.next_param_padded()?;

    Ok(curve_to_vec(point1 + point2))
}

/// multiply a point on an elliptic curve by a scalar value
fn ec_mul<RT: Runtime>(_: &mut System<RT>, input: &[u8], _: PrecompileContext) -> PrecompileResult {
    let input = read_right_pad(input, 96);
    let mut input_params: PaddedChunks<u8, 64> = PaddedChunks::new(&input);
    let point = input_params.next_param_padded()?;

    let scalar = {
        let data = U256::from_big_endian(&input_params.remaining_slice()[..32]);
        Fr::new_mul_factor(data.into())
    };

    Ok(curve_to_vec(point * scalar))
}

/// pairs multple groups of twisted bn curves
fn ec_pairing<RT: Runtime>(
    _: &mut System<RT>,
    input: &[u8],
    _: PrecompileContext,
) -> PrecompileResult {
    fn read_group(input: &[u8]) -> Result<(G1, G2), PrecompileError> {
        let mut i_in = U256Reader::new(input);

        let x = Fq::from_u256(i_in.next_into_param::<U256>()?.into())?;
        let y = Fq::from_u256(i_in.next_into_param::<U256>()?.into())?;

        let twisted_x = {
            let b = Fq::from_u256(i_in.next_into_param::<U256>()?.into())?;
            let a = Fq::from_u256(i_in.next_into_param::<U256>()?.into())?;
            Fq2::new(a, b)
        };
        let twisted_y = {
            let b = Fq::from_u256(i_in.next_into_param::<U256>()?.into())?;
            let a = Fq::from_u256(i_in.next_into_param::<U256>()?.into())?;
            Fq2::new(a, b)
        };

        let twisted = {
            if twisted_x.is_zero() && twisted_y.is_zero() {
                G2::zero()
            } else {
                AffineG2::new(twisted_x, twisted_y)?.into()
            }
        };

        let a = {
            if x.is_zero() && y.is_zero() {
                substrate_bn::G1::zero()
            } else {
                AffineG1::new(x, y)?.into()
            }
        };

        Ok((a, twisted))
    }

    const GROUP_BYTE_LEN: usize = 192;

    if input.len() % GROUP_BYTE_LEN != 0 {
        return Err(PrecompileError::IncorrectInputSize);
    }

    let mut groups = Vec::new();
    for i in 0..input.len() / GROUP_BYTE_LEN {
        let offset = i * GROUP_BYTE_LEN;
        groups.push(read_group(&input[offset..offset + GROUP_BYTE_LEN])?);
    }

    let accumulated = pairing_batch(&groups);

    let paring_success = if accumulated == Gt::one() { U256::one() } else { U256::zero() };
    let mut ret = [0u8; 32];
    paring_success.to_big_endian(&mut ret);
    Ok(ret.to_vec())
}

/// https://eips.ethereum.org/EIPS/eip-152
fn blake2f<RT: Runtime>(
    _: &mut System<RT>,
    input: &[u8],
    _: PrecompileContext,
) -> PrecompileResult {
    if input.len() != 213 {
        return Err(PrecompileError::IncorrectInputSize);
    }
    let mut hasher = near_blake2::VarBlake2b::default();
    let mut rounds = [0u8; 4];

    let mut start = 0;

    // 4 bytes
    rounds.copy_from_slice(&input[..4]);
    start += 4;
    // 64 bytes
    let h = &input[start..start + 64];
    start += 64;
    // 128 bytes
    let m = &input[start..start + 128];
    start += 128;
    // 16 bytes
    let t = &input[start..start + 16];
    start += 16;

    debug_assert_eq!(start, 212, "expected start to be at the last byte");
    let f = match input[start] {
        0 => Ok(false),
        1 => Ok(true),
        _ => Err(PrecompileError::IncorrectInputSize),
    }?;

    let rounds = u32::from_be_bytes(rounds);
    let h = {
        let mut ret = [0u64; 8];
        LE::read_u64_into(h, &mut ret);
        ret
    };
    let m = {
        let mut ret = [0u64; 16];
        LE::read_u64_into(m, &mut ret);
        ret
    };
    let t = {
        let mut ret = [0u64; 2];
        LE::read_u64_into(t, &mut ret);
        ret
    };

    hasher.blake2_f(rounds, h, m, t, f);
    let output = hasher.output().to_vec();
    Ok(output)
}

#[cfg(test)]
mod tests {
    use super::*;
    use fil_actors_runtime::test_utils::MockRuntime;
    use hex_literal::hex;

    #[test]
    fn padding() {
        let input = b"foo bar boxy";
        let mut input = Vec::from(*input);
        for i in 12..64 {
            let mut expected = input.clone();
            expected.resize(i, 0);

            let res = read_right_pad(&input, i);
            assert_eq!(&*res, &expected);

            input.push(0);
        }
    }

    #[test]
    fn bn_recover() {
        let mut rt = MockRuntime::default();
        let mut system = System::create(&mut rt).unwrap();

        let input = &hex!(
            "456e9aea5e197a1f1af7a3e85a3212fa4049a3ba34c2289b4c860fc0b0c64ef3" // h(ash)
            "000000000000000000000000000000000000000000000000000000000000001c" // v (recovery byte)
            // signature
            "9242685bf161793cc25603c231bc2f568eb630ea16aa137d2664ac8038825608" // r
            "4f8ae3bd7535248d0bd448298cc2e2071e56992d0774dc340c368ae950852ada" // s
        );

        let expected = hex!("0000000000000000000000007156526fbd7a3c72969b54f64e42c10fbb768c8a");
        let res = ec_recover(&mut system, input, PrecompileContext::default()).unwrap();
        assert_eq!(&res, &expected);

        let input = &hex!(
            "456e9aea5e197a1f1af7a3e85a3212fa4049a3ba34c2289b4c860fc0b0c64ef3" // h(ash)
            "000000000000000000000000000000000000000000000000000000000000001c" // v (recovery byte)
            // signature
            "0000005bf161793cc25603c231bc2f568eb630ea16aa137d2664ac8038825608" // r
            "4f8ae3bd7535248d0bd448298cc2e2071e56992d0774dc340c368ae950852ada" // s
        );
        // wrong signature
        let res = ec_recover(&mut system, input, PrecompileContext::default()).unwrap();
        assert_eq!(res, Vec::new());

        let input = &hex!(
            "456e9aea5e197a1f1af7a3e85a3212fa4049a3ba34c2289b4c860fc0b0c64ef3" // h(ash)
            "000000000000000000000000000000000000000000000000000000000000000a" // v (recovery byte)
            // signature
            "0000005bf161793cc25603c231bc2f568eb630ea16aa137d2664ac8038825608" // r
            "4f8ae3bd7535248d0bd448298cc2e2071e56992d0774dc340c368ae950852ada" // s
        );
        // invalid recovery byte
        let res = ec_recover(&mut system, input, PrecompileContext::default()).unwrap();
        assert_eq!(res, Vec::new());
    }

    #[test]
    fn sha256() {
        use super::sha256 as hash;
        let input = "foo bar baz boxy".as_bytes();

        let mut rt = MockRuntime::default();
        let mut system = System::create(&mut rt).unwrap();

        let expected = hex!("ace8597929092c14bd028ede7b07727875788c7e130278b5afed41940d965aba");
        let res = hash(&mut system, input, PrecompileContext::default()).unwrap();
        assert_eq!(&res, &expected);
    }

    #[test]
    fn ripemd160() {
        use super::ripemd160 as hash;
        let input = "foo bar baz boxy".as_bytes();

        let mut rt = MockRuntime::default();
        let mut system = System::create(&mut rt).unwrap();

        let expected = hex!("4cd7a0452bd3d682e4cbd5fa90f446d7285b156a");
        let res = hash(&mut system, input, PrecompileContext::default()).unwrap();
        assert_eq!(&res, &expected);
    }

    #[test]
    fn mod_exponent() {
        let input = &hex!(
            "0000000000000000000000000000000000000000000000000000000000000001" // base len
            "0000000000000000000000000000000000000000000000000000000000000001" // exp len
            "0000000000000000000000000000000000000000000000000000000000000001" // mod len
            "08" // base
            "09" // exp
            "0A" // mod
        );

        let mut rt = MockRuntime::default();
        let mut system = System::create(&mut rt).unwrap();

        let expected = hex!("08");
        let res = modexp(&mut system, input, PrecompileContext::default()).unwrap();
        assert_eq!(&res, &expected);

        let input = &hex!(
            "0000000000000000000000000000000000000000000000000000000000000004" // base len
            "0000000000000000000000000000000000000000000000000000000000000002" // exp len
            "0000000000000000000000000000000000000000000000000000000000000006" // mod len
            "12345678" // base
            "1234" // exp
            "012345678910" // mod
        );
        let expected = hex!("00358eac8f30"); // left padding & 230026940208
        let res = modexp(&mut system, input, PrecompileContext::default()).unwrap();
        assert_eq!(&res, &expected);

        let expected = hex!("000000"); // invalid values will just be [0; mod_len]
        let input = &hex!(
            "0000000000000000000000000000000000000000000000000000000000000001" // base len
            "0000000000000000000000000000000000000000000000000000000000000002" // exp len
            "0000000000000000000000000000000000000000000000000000000000000003" // mod len
            "01" // base
            "02" // exp
            "03" // mod
        );
        // input smaller than expected
        let res = modexp(&mut system, input, PrecompileContext::default()).unwrap();
        assert_eq!(&res, &expected);

        let input = &hex!(
            "0000000000000000000000000000000000000000000000000000000000000001" // base len
            "0000000000000000000000000000000000000000000000000000000000000001" // exp len
            "0000000000000000000000000000000000000000000000000000000000000000" // mod len
            "08" // base
            "09" // exp
        );
        // no mod is invalid
        let res = modexp(&mut system, input, PrecompileContext::default()).unwrap();
        assert_eq!(res, Vec::new());
    }

    // bn tests borrowed from https://github.com/bluealloy/revm/blob/26540bf5b29de6e7c8020c4c1880f8a97d1eadc9/crates/revm_precompiles/src/bn128.rs
    mod bn {
        use super::{GroupError, MockRuntime};
        use crate::interpreter::{
            precompiles::{ec_add, ec_mul, ec_pairing, PrecompileContext, PrecompileError},
            System,
        };

        #[test]
        fn bn_add() {
            let mut rt = MockRuntime::default();
            let mut system = System::create(&mut rt).unwrap();

            let input = hex::decode(
                "\
                 18b18acfb4c2c30276db5411368e7185b311dd124691610c5d3b74034e093dc9\
                 063c909c4720840cb5134cb9f59fa749755796819658d32efc0d288198f37266\
                 07c2b7f58a84bd6145f00c9c2bc0bb1a187f20ff2c92963a88019e7c6a014eed\
                 06614e20c147e940f2d70da3f74c9a17df361706a4485c742bd6788478fa17d7",
            )
            .unwrap();
            let expected = hex::decode(
                "\
                2243525c5efd4b9c3d3c45ac0ca3fe4dd85e830a4ce6b65fa1eeaee202839703\
                301d1d33be6da8e509df21cc35964723180eed7532537db9ae5e7d48f195c915",
            )
            .unwrap();
            let res = ec_add(&mut system, &input, PrecompileContext::default()).unwrap();
            assert_eq!(res, expected);
            // zero sum test
            let input = hex::decode(
                "\
                0000000000000000000000000000000000000000000000000000000000000000\
                0000000000000000000000000000000000000000000000000000000000000000\
                0000000000000000000000000000000000000000000000000000000000000000\
                0000000000000000000000000000000000000000000000000000000000000000",
            )
            .unwrap();
            let expected = hex::decode(
                "\
                0000000000000000000000000000000000000000000000000000000000000000\
                0000000000000000000000000000000000000000000000000000000000000000",
            )
            .unwrap();
            let res = ec_add(&mut system, &input, PrecompileContext::default()).unwrap();
            assert_eq!(res, expected);

            // no input test
            let input = [];
            let expected = hex::decode(
                "\
                0000000000000000000000000000000000000000000000000000000000000000\
                0000000000000000000000000000000000000000000000000000000000000000",
            )
            .unwrap();
            let res = ec_add(&mut system, &input, PrecompileContext::default()).unwrap();
            assert_eq!(res, expected);
            // point not on curve fail
            let input = hex::decode(
                "\
                1111111111111111111111111111111111111111111111111111111111111111\
                1111111111111111111111111111111111111111111111111111111111111111\
                1111111111111111111111111111111111111111111111111111111111111111\
                1111111111111111111111111111111111111111111111111111111111111111",
            )
            .unwrap();
            let res = ec_add(&mut system, &input, PrecompileContext::default());
            assert!(matches!(res, Err(PrecompileError::EcGroupErr(GroupError::NotOnCurve))));
        }

        #[test]
        fn bn_mul() {
            let mut rt = MockRuntime::default();
            let mut system = System::create(&mut rt).unwrap();

            let input = hex::decode(
                "\
                2bd3e6d0f3b142924f5ca7b49ce5b9d54c4703d7ae5648e61d02268b1a0a9fb7\
                21611ce0a6af85915e2f1d70300909ce2e49dfad4a4619c8390cae66cefdb204\
                00000000000000000000000000000000000000000000000011138ce750fa15c2",
            )
            .unwrap();
            let expected = hex::decode(
                "\
                070a8d6a982153cae4be29d434e8faef8a47b274a053f5a4ee2a6c9c13c31e5c\
                031b8ce914eba3a9ffb989f9cdd5b0f01943074bf4f0f315690ec3cec6981afc",
            )
            .unwrap();
            let res = ec_mul(&mut system, &input, PrecompileContext::default()).unwrap();
            assert_eq!(res, expected);

            // out of gas test
            let input = hex::decode(
                "\
                0000000000000000000000000000000000000000000000000000000000000000\
                0000000000000000000000000000000000000000000000000000000000000000\
                0200000000000000000000000000000000000000000000000000000000000000",
            )
            .unwrap();
            let res = ec_mul(&mut system, &input, PrecompileContext::default()).unwrap();
            assert_eq!(&res, &vec![0; 64]);

            // no input test
            let input = [0u8; 0];
            let expected = hex::decode(
                "\
                0000000000000000000000000000000000000000000000000000000000000000\
                0000000000000000000000000000000000000000000000000000000000000000",
            )
            .unwrap();
            let res = ec_mul(&mut system, &input, PrecompileContext::default()).unwrap();
            assert_eq!(res, expected);
            // point not on curve fail
            let input = hex::decode(
                "\
                1111111111111111111111111111111111111111111111111111111111111111\
                1111111111111111111111111111111111111111111111111111111111111111\
                0f00000000000000000000000000000000000000000000000000000000000000",
            )
            .unwrap();
            let res = ec_mul(&mut system, &input, PrecompileContext::default());
            assert!(matches!(res, Err(PrecompileError::EcGroupErr(GroupError::NotOnCurve))));
        }

        #[test]
        fn bn_pair() {
            let mut rt = MockRuntime::default();
            let mut system = System::create(&mut rt).unwrap();

            let input = hex::decode(
                "\
                1c76476f4def4bb94541d57ebba1193381ffa7aa76ada664dd31c16024c43f59\
                3034dd2920f673e204fee2811c678745fc819b55d3e9d294e45c9b03a76aef41\
                209dd15ebff5d46c4bd888e51a93cf99a7329636c63514396b4a452003a35bf7\
                04bf11ca01483bfa8b34b43561848d28905960114c8ac04049af4b6315a41678\
                2bb8324af6cfc93537a2ad1a445cfd0ca2a71acd7ac41fadbf933c2a51be344d\
                120a2a4cf30c1bf9845f20c6fe39e07ea2cce61f0c9bb048165fe5e4de877550\
                111e129f1cf1097710d41c4ac70fcdfa5ba2023c6ff1cbeac322de49d1b6df7c\
                2032c61a830e3c17286de9462bf242fca2883585b93870a73853face6a6bf411\
                198e9393920d483a7260bfb731fb5d25f1aa493335a9e71297e485b7aef312c2\
                1800deef121f1e76426a00665e5c4479674322d4f75edadd46debd5cd992f6ed\
                090689d0585ff075ec9e99ad690c3395bc4b313370b38ef355acdadcd122975b\
                12c85ea5db8c6deb4aab71808dcb408fe3d1e7690c43d37b4ce6cc0166fa7daa",
            )
            .unwrap();

            let expected =
                hex::decode("0000000000000000000000000000000000000000000000000000000000000001")
                    .unwrap();

            let res = ec_pairing(&mut system, &input, PrecompileContext::default()).unwrap();
            assert_eq!(res, expected);

            // out of gas test
            let input = hex::decode(
                "\
                1c76476f4def4bb94541d57ebba1193381ffa7aa76ada664dd31c16024c43f59\
                3034dd2920f673e204fee2811c678745fc819b55d3e9d294e45c9b03a76aef41\
                209dd15ebff5d46c4bd888e51a93cf99a7329636c63514396b4a452003a35bf7\
                04bf11ca01483bfa8b34b43561848d28905960114c8ac04049af4b6315a41678\
                2bb8324af6cfc93537a2ad1a445cfd0ca2a71acd7ac41fadbf933c2a51be344d\
                120a2a4cf30c1bf9845f20c6fe39e07ea2cce61f0c9bb048165fe5e4de877550\
                111e129f1cf1097710d41c4ac70fcdfa5ba2023c6ff1cbeac322de49d1b6df7c\
                2032c61a830e3c17286de9462bf242fca2883585b93870a73853face6a6bf411\
                198e9393920d483a7260bfb731fb5d25f1aa493335a9e71297e485b7aef312c2\
                1800deef121f1e76426a00665e5c4479674322d4f75edadd46debd5cd992f6ed\
                090689d0585ff075ec9e99ad690c3395bc4b313370b38ef355acdadcd122975b\
                12c85ea5db8c6deb4aab71808dcb408fe3d1e7690c43d37b4ce6cc0166fa7daa",
            )
            .unwrap();
            let res = ec_pairing(&mut system, &input, PrecompileContext::default()).unwrap();
            assert_eq!(res, expected);
            // no input test
            let input = [0u8; 0];
            let expected =
                hex::decode("0000000000000000000000000000000000000000000000000000000000000001")
                    .unwrap();
            let res = ec_pairing(&mut system, &input, PrecompileContext::default()).unwrap();
            assert_eq!(res, expected);
            // point not on curve fail
            let input = hex::decode(
                "\
                1111111111111111111111111111111111111111111111111111111111111111\
                1111111111111111111111111111111111111111111111111111111111111111\
                1111111111111111111111111111111111111111111111111111111111111111\
                1111111111111111111111111111111111111111111111111111111111111111\
                1111111111111111111111111111111111111111111111111111111111111111\
                1111111111111111111111111111111111111111111111111111111111111111",
            )
            .unwrap();
            let res = ec_pairing(&mut system, &input, PrecompileContext::default());
            assert!(matches!(res, Err(PrecompileError::EcGroupErr(GroupError::NotOnCurve))));
            // invalid input length
            let input = hex::decode(
                "\
                1111111111111111111111111111111111111111111111111111111111111111\
                1111111111111111111111111111111111111111111111111111111111111111\
                111111111111111111111111111111\
            ",
            )
            .unwrap();
            let res = ec_pairing(&mut system, &input, PrecompileContext::default());
            assert!(matches!(res, Err(PrecompileError::IncorrectInputSize)));
        }
    }

    // https://eips.ethereum.org/EIPS/eip-152#test-cases
    #[test]
    fn blake2() {
        use super::blake2f;
        let mut rt = MockRuntime::default();
        let mut system = System::create(&mut rt).unwrap();

        // // helper to turn EIP test cases into something readable
        // fn test_case_formatter(mut remaining: impl ToString) {
        //     let mut rounds = remaining.to_string();
        //     let mut h = rounds.split_off(2*4);
        //     let mut m = h.split_off(2*64);
        //     let mut t_0 = m.split_off(2*128);
        //     let mut t_1 = t_0.split_off(2*8);
        //     let mut f = t_1.split_off(2*8);

        //     println!("
        //         \"{rounds}\"
        //         \"{h}\"
        //         \"{m}\"
        //         \"{t_0}\"
        //         \"{t_1}\"
        //         \"{f}\"
        //     ")
        // }

        // T0 invalid input len
        assert!(matches!(
            blake2f(&mut system, &[], PrecompileContext::default()),
            Err(PrecompileError::IncorrectInputSize)
        ));

        // T1 too small
        let input = &hex!(
            "00000c"
            "48c9bdf267e6096a3ba7ca8485ae67bb2bf894fe72f36e3cf1361d5f3af54fa5d182e6ad7f520e511f6c3e2b8c68059b6bbd41fbabd9831f79217e1319cde05b"
            "6162630000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000"
            "0300000000000000"
            "0000000000000000"
            "02"
        );
        assert!(matches!(
            blake2f(&mut system, input, PrecompileContext::default()),
            Err(PrecompileError::IncorrectInputSize)
        ));

        // T2 too large
        let input = &hex!(
            "000000000c"
            "48c9bdf267e6096a3ba7ca8485ae67bb2bf894fe72f36e3cf1361d5f3af54fa5d182e6ad7f520e511f6c3e2b8c68059b6bbd41fbabd9831f79217e1319cde05b"
            "6162630000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000"
            "0300000000000000"
            "0000000000000000"
            "02"
        );
        assert!(matches!(
            blake2f(&mut system, input, PrecompileContext::default()),
            Err(PrecompileError::IncorrectInputSize)
        ));

        // T3 final block indicator invalid
        let input = &hex!(
            "0000000c"
            "48c9bdf267e6096a3ba7ca8485ae67bb2bf894fe72f36e3cf1361d5f3af54fa5d182e6ad7f520e511f6c3e2b8c68059b6bbd41fbabd9831f79217e1319cde05b"
            "6162630000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000"
            "0300000000000000"
            "0000000000000000"
            "02"
        );
        assert!(matches!(
            blake2f(&mut system, input, PrecompileContext::default()),
            Err(PrecompileError::IncorrectInputSize)
        ));

        // outputs

        // T4
        let expected = hex!("08c9bcf367e6096a3ba7ca8485ae67bb2bf894fe72f36e3cf1361d5f3af54fa5d282e6ad7f520e511f6c3e2b8c68059b9442be0454267ce079217e1319cde05b");
        let input = &hex!(
            "00000000"
            "48c9bdf267e6096a3ba7ca8485ae67bb2bf894fe72f36e3cf1361d5f3af54fa5d182e6ad7f520e511f6c3e2b8c68059b6bbd41fbabd9831f79217e1319cde05b"
            "6162630000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000"
            "0300000000000000"
            "0000000000000000"
            "01"
        );
        assert!(
            matches!(blake2f(&mut system, input, PrecompileContext::default()), Ok(v) if v == expected)
        );

        // T5
        let expected = &hex!("ba80a53f981c4d0d6a2797b69f12f6e94c212f14685ac4b74b12bb6fdbffa2d17d87c5392aab792dc252d5de4533cc9518d38aa8dbf1925ab92386edd4009923");
        let input = &hex!(
            "0000000c"
            "48c9bdf267e6096a3ba7ca8485ae67bb2bf894fe72f36e3cf1361d5f3af54fa5d182e6ad7f520e511f6c3e2b8c68059b6bbd41fbabd9831f79217e1319cde05b"
            "6162630000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000"
            "0300000000000000"
            "0000000000000000"
            "01"
        );
        assert!(
            matches!(blake2f(&mut system, input, PrecompileContext::default()), Ok(v) if v == expected)
        );

        // T6
        let expected = &hex!("75ab69d3190a562c51aef8d88f1c2775876944407270c42c9844252c26d2875298743e7f6d5ea2f2d3e8d226039cd31b4e426ac4f2d3d666a610c2116fde4735");
        let input = &hex!(
            "0000000c"
            "48c9bdf267e6096a3ba7ca8485ae67bb2bf894fe72f36e3cf1361d5f3af54fa5d182e6ad7f520e511f6c3e2b8c68059b6bbd41fbabd9831f79217e1319cde05b"
            "6162630000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000"
            "0300000000000000"
            "0000000000000000"
            "00"
        );
        assert!(
            matches!(blake2f(&mut system, input, PrecompileContext::default()), Ok(v) if v == expected)
        );

        // T7
        let expected = &hex!("b63a380cb2897d521994a85234ee2c181b5f844d2c624c002677e9703449d2fba551b3a8333bcdf5f2f7e08993d53923de3d64fcc68c034e717b9293fed7a421");
        let input = &hex!(
            "00000001"
            "48c9bdf267e6096a3ba7ca8485ae67bb2bf894fe72f36e3cf1361d5f3af54fa5d182e6ad7f520e511f6c3e2b8c68059b6bbd41fbabd9831f79217e1319cde05b"
            "6162630000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000"
            "0300000000000000"
            "0000000000000000"
            "01"
        );
        assert!(
            matches!(blake2f(&mut system, input, PrecompileContext::default()), Ok(v) if v == expected)
        );

        // T8
        // NOTE:
        //  original test case ran ffffffff rounds of blake2b
        //  with an expected output of fc59093aafa9ab43daae0e914c57635c5402d8e3d2130eb9b3cc181de7f0ecf9b22bf99a7815ce16419e200e01846e6b5df8cc7703041bbceb571de6631d2615
        //  I ran this successfully while grabbing a cup of coffee, so if you fee like wasting u32::MAX rounds of hash time, (25-ish min on Ryzen5 2600) you can test it as such.
        //  For my and CI's sanity however, we are capping it at 0000ffff.
        let expected = &hex!("183ed9b1e5594bcdd715a4e4fd7b0dc2eaa2ef9bda48242af64c687081142156621bc94bb2d5aa99d83c2f1a5d9c426e1b6a1755a5e080f6217e2a5f3b9c4624");
        let input = &hex!(
            "0000ffff"
            "48c9bdf267e6096a3ba7ca8485ae67bb2bf894fe72f36e3cf1361d5f3af54fa5d182e6ad7f520e511f6c3e2b8c68059b6bbd41fbabd9831f79217e1319cde05b"
            "6162630000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000"
            "0300000000000000"
            "0000000000000000"
            "01"
        );
        assert!(
            matches!(blake2f(&mut system, input, PrecompileContext::default()), Ok(v) if v == expected)
        );
    }
}
