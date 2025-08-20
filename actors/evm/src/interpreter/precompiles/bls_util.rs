use super::PrecompileError;
use blst::{
    // Basic types
    blst_bendian_from_fp,
    blst_fp,
    blst_fp_from_bendian,
    blst_fp2,
    blst_p1,
    blst_p1_affine,
    blst_p1_affine_in_g1,
    blst_p1_affine_is_inf,
    blst_p1_affine_on_curve,
    blst_p1_from_affine,
    blst_p1_mult,
    blst_p1_to_affine,
    blst_p2,
    blst_p2_affine,
    blst_p2_affine_in_g2,
    blst_p2_affine_is_inf,
    blst_p2_affine_on_curve,
    blst_p2_from_affine,
    blst_p2_mult,
    blst_p2_to_affine,
    blst_scalar,
    blst_scalar_from_bendian,
};

pub const G1_INPUT_LENGTH: usize = 128;
pub const G1_ADD_INPUT_LENGTH: usize = G1_INPUT_LENGTH * 2;
pub const G1_OUTPUT_LENGTH: usize = 128;
pub const PADDING_LENGTH: usize = 16;
pub const G1_MSM_INPUT_LENGTH: usize = 160;
pub const SCALAR_LENGTH: usize = 32;
pub const G2_ADD_INPUT_LENGTH: usize = 512;
pub const G2_INPUT_ITEM_LENGTH: usize = 256;
pub const G2_OUTPUT_LENGTH: usize = 256;
pub const G2_MSM_INPUT_LENGTH: usize = 288;
pub const PADDED_FP_LENGTH: usize = 64;
pub const PADDED_FP2_LENGTH: usize = 2 * PADDED_FP_LENGTH;
pub const PADDED_G1_LENGTH: usize = 2 * PADDED_FP_LENGTH;
pub const PADDED_G2_LENGTH: usize = 2 * PADDED_FP2_LENGTH;
pub const PAIRING_INPUT_LENGTH: usize = PADDED_G1_LENGTH + PADDED_G2_LENGTH;

/// FP_LENGTH specifies the number of bytes needed to represent an
/// Fp element. This is an element in the base field of BLS12-381.
///
/// Note: The base field is used to define G1 and G2 elements.
pub const FP_LENGTH: usize = 48;

/// SCALAR_LENGTH_BITS specifies the number of bits needed to represent an Fr element.
/// Note Fr in EIP 2537 notation is Fq, the BLS 381 Scalar field
/// This is an element in the scalar field of BLS12-381.
pub const SCALAR_LENGTH_BITS: usize = SCALAR_LENGTH * 8;

// Big-endian non-Montgomery form.
const MODULUS_REPR: [u8; 48] = [
    0x1a, 0x01, 0x11, 0xea, 0x39, 0x7f, 0xe6, 0x9a, 0x4b, 0x1b, 0xa7, 0xb6, 0x43, 0x4b, 0xac, 0xd7,
    0x64, 0x77, 0x4b, 0x84, 0xf3, 0x85, 0x12, 0xbf, 0x67, 0x30, 0xd2, 0xa0, 0xf6, 0xb0, 0xf6, 0x24,
    0x1e, 0xab, 0xff, 0xfe, 0xb1, 0x53, 0xff, 0xff, 0xb9, 0xfe, 0xff, 0xff, 0xff, 0xff, 0xaa, 0xab,
];
use substrate_bn::CurveError;

#[inline]
pub fn p1_to_affine(p: &blst_p1) -> blst_p1_affine {
    let mut p_affine = blst_p1_affine::default();
    // SAFETY: both inputs are valid blst types
    unsafe { blst_p1_to_affine(&mut p_affine, p) };
    p_affine
}

#[inline]
pub fn p1_from_affine(p_affine: &blst_p1_affine) -> blst_p1 {
    let mut p = blst_p1::default();
    // SAFETY: both inputs are valid blst types
    unsafe { blst_p1_from_affine(&mut p, p_affine) };
    p
}

#[inline]
pub fn p2_to_affine(p: &blst_p2) -> blst_p2_affine {
    let mut p_affine = blst_p2_affine::default();
    // SAFETY: both inputs are valid blst types
    unsafe { blst_p2_to_affine(&mut p_affine, p) };
    p_affine
}

#[inline]
pub fn p2_from_affine(p_affine: &blst_p2_affine) -> blst_p2 {
    let mut p = blst_p2::default();
    // SAFETY: both inputs are valid blst types
    unsafe { blst_p2_from_affine(&mut p, p_affine) };
    p
}

pub fn read_scalar(input: &[u8]) -> Result<blst_scalar, PrecompileError> {
    if input.len() != SCALAR_LENGTH {
        return Err(PrecompileError::IncorrectInputSize);
    }

    let mut out = blst_scalar::default();
    // SAFETY: `input` length is checked previously, out is a blst value.
    unsafe {
        // Note: We do not use `blst_scalar_fr_check` here because, from EIP-2537:
        //
        // * The corresponding integer is not required to be less than or equal than main subgroup
        // order `q`.
        blst_scalar_from_bendian(&mut out, input.as_ptr())
    };

    Ok(out)
}

/// Performs a G1 scalar multiplication
///
/// Takes a G1 point in affine form and a scalar, and returns the result
/// of the scalar multiplication in affine form
///
/// Note: The scalar is expected to be in Big Endian format.
#[inline]
pub fn p1_scalar_mul(p: &blst_p1_affine, scalar: &blst_scalar) -> blst_p1_affine {
    // Convert point to Jacobian coordinates
    let p_jacobian = p1_from_affine(p);

    let mut result = blst_p1::default();

    // SAFETY: all inputs are valid blst types
    unsafe { blst_p1_mult(&mut result, &p_jacobian, scalar.b.as_ptr(), scalar.b.len() * 8) };

    // Convert result back to affine coordinates
    p1_to_affine(&result)
}

/// Performs a G2 scalar multiplication
///
/// Takes a G2 point in affine form and a scalar, and returns the result
/// of the scalar multiplication in affine form
///
/// Note: The scalar is expected to be in Big Endian format.
#[inline]
pub fn p2_scalar_mul(p: &blst_p2_affine, scalar: &blst_scalar) -> blst_p2_affine {
    // Convert point to Jacobian coordinates
    let p_jacobian = p2_from_affine(p);

    let mut result = blst_p2::default();
    // SAFETY: all inputs are valid blst types
    unsafe { blst_p2_mult(&mut result, &p_jacobian, scalar.b.as_ptr(), scalar.b.len() * 8) };

    // Convert result back to affine coordinates
    p2_to_affine(&result)
}

/// Encodes a G2 point in affine format into byte slice with padded elements.
/// G2 points have two coordinates (x,y) where each coordinate is a complex number (real,imaginary)
/// So we need to encode 4 field elements total: x.re, x.im, y.re, y.im
pub(super) fn encode_g2_point(input: &blst_p2_affine) -> Vec<u8> {
    // Create output buffer with space for all coordinates (4 * 64 bytes)
    let mut out = vec![0u8; PADDED_G2_LENGTH];
    // Encode x coordinate
    // Real part (x.fp[0])
    fp_to_bytes(&mut out[..PADDED_FP_LENGTH], &input.x.fp[0]);
    // Imaginary part (x.fp[1])
    fp_to_bytes(&mut out[PADDED_FP_LENGTH..2 * PADDED_FP_LENGTH], &input.x.fp[1]);

    // Encode y coordinate
    // Real part (y.fp[0])
    fp_to_bytes(&mut out[2 * PADDED_FP_LENGTH..3 * PADDED_FP_LENGTH], &input.y.fp[0]);
    // Imaginary part (y.fp[1])
    fp_to_bytes(&mut out[3 * PADDED_FP_LENGTH..4 * PADDED_FP_LENGTH], &input.y.fp[1]);

    out
}

/// Extracts a G2 point in Affine format from a 256 byte slice representation.
///
/// **Note**: This function will perform a G2 subgroup check if `subgroup_check` is set to `true`.
///
/// Subgroup checks are required for:
/// - Scalar multiplication
/// - Multi-scalar multiplication (MSM)
/// - Pairing operations
///
/// But not required for:
/// - Point addition
/// - Point negation
pub(super) fn extract_g2_input(
    input: &[u8],
    subgroup_check: bool,
) -> Result<blst_p2_affine, PrecompileError> {
    // Check input length (256 bytes = 4 * 64 bytes for x.re, x.im, y.re, y.im)
    if input.len() != G2_INPUT_ITEM_LENGTH {
        return Err(PrecompileError::IncorrectInputSize);
    }

    let [x_re, x_im, y_re, y_im] = remove_g2_padding(input)?;
    // Convert bytes to point
    let point = decode_g2_on_curve(x_re, x_im, y_re, y_im)?;

    if subgroup_check {
        // Subgroup check (more expensive but required for certain operations)
        // Verifies that the point has the correct order and is in G2
        // SAFETY: point is properly initialized above
        unsafe {
            if !blst_p2_affine_in_g2(&point) {
                return Err(PrecompileError::InvalidInput);
            }
        }
    } else {
        // Basic curve check (less expensive, sufficient for addition)
        // Only verifies that the point is on the curve
        // SAFETY: point is properly initialized above
        if unsafe { !blst_p2_affine_on_curve(&point) } {
            return Err(PrecompileError::InvalidInput);
        }
    }

    Ok(point)
}

/// https://eips.ethereum.org/EIPS/eip-2537
/// Encodes a single finite field element into byte slice with padding.
pub(super) fn fp_to_bytes(out: &mut [u8], input: *const blst_fp) {
    if out.len() != PADDED_FP_LENGTH {
        return;
    }
    let (padding, rest) = out.split_at_mut(PADDING_LENGTH);
    padding.fill(0);
    // SAFETY: Out length is checked previously, `input` is a blst value.
    unsafe { blst_bendian_from_fp(rest.as_mut_ptr(), input) };
}

/// Extracts a G1 point in Affine format from a 128 byte slice representation.
pub fn extract_g1_input(
    input: &[u8],
    subgroup_check: bool,
) -> Result<blst_p1_affine, PrecompileError> {
    if input.len() != G1_INPUT_LENGTH {
        return Err(PrecompileError::IncorrectInputSize);
    }

    let [x_bytes, y_bytes] = remove_g1_padding(input)?;

    let point = decode_g1_on_curve(x_bytes, y_bytes)?;

    // Check if point is on curve (no subgroup check needed for addition)
    if subgroup_check && unsafe { !blst_p1_affine_in_g1(&point) } {
        return Err(PrecompileError::InvalidInput);
    }
    Ok(point)
}

/// Removes zeros with which the precompile inputs are left padded to 64 bytes.
pub fn remove_padding(input: &[u8]) -> Result<&[u8; 48], PrecompileError> {
    if input.len() != PADDED_FP_LENGTH {
        return Err(PrecompileError::IncorrectInputSize);
    }
    let (padding, unpadded) = input.split_at(PADDING_LENGTH);
    if !padding.iter().all(|&x| x == 0) {
        return Err(PrecompileError::InvalidInput);
    }
    unpadded.try_into().map_err(|_| PrecompileError::IncorrectInputSize)
}

/// Encodes a G1 point in affine format into byte slice with padded elements.
pub fn encode_g1_point(input: *const blst_p1_affine) -> Vec<u8> {
    let mut out = vec![0u8; G1_OUTPUT_LENGTH];
    // SAFETY: Out comes from fixed length array, input is a blst value.
    unsafe {
        fp_to_bytes(&mut out[..PADDED_FP_LENGTH], &(*input).x);
        fp_to_bytes(&mut out[PADDED_FP_LENGTH..], &(*input).y);
    }
    out
}

/// Returns a `blst_p1_affine` from the provided byte slices, which represent the x and y
/// affine coordinates of the point.
///
/// Note: Coordinates are expected to be in Big Endian format.
///
/// - If the x or y coordinate do not represent a canonical field element, an error is returned.
///   See [read_fp] for more information.
/// - If the point is not on the curve, an error is returned.
fn decode_g1_on_curve(
    p0_x: &[u8; FP_LENGTH],
    p0_y: &[u8; FP_LENGTH],
) -> Result<blst_p1_affine, PrecompileError> {
    let out = blst_p1_affine { x: read_fp(p0_x)?, y: read_fp(p0_y)? };

    // From EIP-2537:
    //
    // Error cases:
    //
    // * An input is neither a point on the G1 elliptic curve nor the infinity point
    //
    // SAFETY: Out is a blst value.
    if unsafe { !blst_p1_affine_on_curve(&out) } {
        return Err(PrecompileError::EcErr(CurveError::NotMember));
    }

    Ok(out)
}

/// remove_g1_padding removes the padding applied to the Fp elements that constitute the
/// encoded G1 element.
pub(super) fn remove_g1_padding(input: &[u8]) -> Result<[&[u8; FP_LENGTH]; 2], PrecompileError> {
    if input.len() != PADDED_G1_LENGTH {
        return Err(PrecompileError::IncorrectInputSize);
    }

    let x = remove_padding(&input[..PADDED_FP_LENGTH])?;
    let y = remove_padding(&input[PADDED_FP_LENGTH..PADDED_G1_LENGTH])?;
    Ok([x, y])
}

/// Returns a `blst_p2_affine` from the provided byte slices, which represent the x and y
/// affine coordinates of the point.
///
/// Note: Coordinates are expected to be in Big Endian format.
///
/// - If the x or y coordinate do not represent a canonical field element, an error is returned.
///   See [read_fp2] for more information.
/// - If the point is not on the curve, an error is returned.
fn decode_g2_on_curve(
    x1: &[u8; FP_LENGTH],
    x2: &[u8; FP_LENGTH],
    y1: &[u8; FP_LENGTH],
    y2: &[u8; FP_LENGTH],
) -> Result<blst_p2_affine, PrecompileError> {
    let out = blst_p2_affine { x: read_fp2(x1, x2)?, y: read_fp2(y1, y2)? };

    // From EIP-2537:
    //
    // Error cases:
    //
    // * An input is neither a point on the G2 elliptic curve nor the infinity point
    //
    // SAFETY: Out is a blst value.
    if unsafe { !blst_p2_affine_on_curve(&out) } {
        return Err(PrecompileError::EcErr(CurveError::NotMember));
    }

    Ok(out)
}

/// remove_g2_padding removes the padding applied to the Fp elements that constitute the
/// encoded G2 element.
pub(super) fn remove_g2_padding(input: &[u8]) -> Result<[&[u8; FP_LENGTH]; 4], PrecompileError> {
    if input.len() != PADDED_G2_LENGTH {
        return Err(PrecompileError::IncorrectInputSize);
    }

    let mut input_fps = [&[0; FP_LENGTH]; 4];
    for i in 0..4 {
        input_fps[i] = remove_padding(&input[i * PADDED_FP_LENGTH..(i + 1) * PADDED_FP_LENGTH])?;
    }
    Ok(input_fps)
}
/// Creates a blst_fp2 element from two field elements.
///
/// Field elements are expected to be in Big Endian format.
/// Returns an error if either of the input field elements is not canonical.
pub(super) fn read_fp2(
    input_1: &[u8; FP_LENGTH],
    input_2: &[u8; FP_LENGTH],
) -> Result<blst_fp2, PrecompileError> {
    let fp_1 = read_fp(input_1)?;
    let fp_2 = read_fp(input_2)?;

    let fp2 = blst_fp2 { fp: [fp_1, fp_2] };

    Ok(fp2)
}
/// Checks whether or not the input represents a canonical field element
/// returning the field element if successful.
///
/// Note: The field element is expected to be in big endian format.
pub fn read_fp(input: &[u8; FP_LENGTH]) -> Result<blst_fp, PrecompileError> {
    if !is_valid_be(input) {
        return Err(PrecompileError::EcErr(CurveError::NotMember));
    }
    let mut fp = blst_fp::default();
    // SAFETY: `input` has fixed length, and `fp` is a blst value.
    unsafe {
        // This performs the check for canonical field elements
        blst_fp_from_bendian(&mut fp, input.as_ptr());
    }

    Ok(fp)
}

/// Checks if the input is a valid big-endian representation of a field element.
fn is_valid_be(input: &[u8; 48]) -> bool {
    *input < MODULUS_REPR
}

/// Trait to check if an affine point is the point at infinity.
pub trait IsInfinity {
    fn is_infinity(&self) -> bool;
}

impl IsInfinity for blst_p1_affine {
    #[inline]
    fn is_infinity(&self) -> bool {
        // # Safety
        // Guaranteed valid by extract_g1_input
        unsafe { blst_p1_affine_is_inf(self) }
    }
}

impl IsInfinity for blst_p2_affine {
    #[inline]
    fn is_infinity(&self) -> bool {
        // # Safety
        // Guaranteed valid by extract_g2_input
        unsafe { blst_p2_affine_is_inf(self) }
    }
}

/// Generic utility to check if an affine point is at infinity.
#[inline]
pub fn is_infinity<T: IsInfinity>(p: &T) -> bool {
    p.is_infinity()
}
