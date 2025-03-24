use super::PrecompileError;
use blst::{
    // Basic types
    blst_fp,
    blst_fp2,
    blst_p1_affine,
    blst_p2_affine,
    blst_scalar,

    // Unsafe functions needed for point operations
    blst_bendian_from_fp,
    blst_fp_from_bendian,
    blst_p1_affine_in_g1,
    blst_p1_affine_on_curve,
    blst_p2_affine_in_g2,
    blst_p2_affine_on_curve,
    blst_scalar_from_bendian,
};

pub const G1_INPUT_LENGTH: usize = 128;
pub const G1_ADD_INPUT_LENGTH: usize = G1_INPUT_LENGTH * 2;
pub const G1_OUTPUT_LENGTH: usize = 128;
pub const PADDING_LENGTH: usize = 16;
pub const G1_MSM_INPUT_LENGTH: usize = 160;
pub const G1_INPUT_ITEM_LENGTH: usize = 128;
pub const SCALAR_LENGTH: usize = 32;
pub const NBITS: usize = 255; 
pub const G2_ADD_INPUT_LENGTH: usize = 512;
pub const G2_INPUT_ITEM_LENGTH: usize = 256;
pub const G2_OUTPUT_LENGTH: usize = 256;
pub const G2_MSM_INPUT_LENGTH: usize = 288;
pub const PADDED_FP_LENGTH: usize = 64;
pub const PADDED_FP2_LENGTH: usize = 2 * PADDED_FP_LENGTH;
pub const PADDED_G1_LENGTH: usize = 2 * PADDED_FP_LENGTH;
pub const PADDED_G2_LENGTH: usize = 2 * PADDED_FP2_LENGTH;
pub const PAIRING_INPUT_LENGTH: usize = PADDED_G1_LENGTH + PADDED_G2_LENGTH;


/// Encodes a G2 point in affine format into byte slice with padded elements.
/// G2 points have two coordinates (x,y) where each coordinate is a complex number (real,imaginary)
/// So we need to encode 4 field elements total: x.re, x.im, y.re, y.im
pub(super) fn encode_g2_point(input: &blst_p2_affine) -> Vec<u8> {
    // Create output buffer with space for all coordinates (4 * 64 bytes)
    let mut out = vec![0u8; G2_OUTPUT_LENGTH];

    // Encode x coordinate
    // Real part (x.fp[0])
    fp_to_bytes(&mut out[..PADDED_FP_LENGTH], &input.x.fp[0]);
    // Imaginary part (x.fp[1]) 
    fp_to_bytes(
        &mut out[PADDED_FP_LENGTH..2 * PADDED_FP_LENGTH],
        &input.x.fp[1],
    );

    // Encode y coordinate
    // Real part (y.fp[0])
    fp_to_bytes(
        &mut out[2 * PADDED_FP_LENGTH..3 * PADDED_FP_LENGTH],
        &input.y.fp[0],
    );
    // Imaginary part (y.fp[1])
    fp_to_bytes(
        &mut out[3 * PADDED_FP_LENGTH..4 * PADDED_FP_LENGTH],
        &input.y.fp[1],
    );

    out
}

/// Convert field elements from byte slices into a `blst_p2_affine` point.
/// Takes four 48-byte arrays representing:
/// - x1: real part of x coordinate
/// - x2: imaginary part of x coordinate
/// - y1: real part of y coordinate
/// - y2: imaginary part of y coordinate
pub(super) fn decode_and_check_g2(
    x1: &[u8; 48], // x.re
    x2: &[u8; 48], // x.im
    y1: &[u8; 48], // y.re
    y2: &[u8; 48], // y.im
) -> Result<blst_p2_affine, PrecompileError> {
    Ok(blst_p2_affine {
        // Create x coordinate as complex number
        x: check_canonical_fp2(x1, x2)?,
        // Create y coordinate as complex number
        y: check_canonical_fp2(y1, y2)?,
    })
}

/// Helper function to create and validate an Fp2 element from two Fp elements
pub fn check_canonical_fp2(
    input_1: &[u8; 48],
    input_2: &[u8; 48],
) -> Result<blst_fp2, PrecompileError> {
    let fp_1 = fp_from_bendian(input_1)?;
    let fp_2 = fp_from_bendian(input_2)?;

    let fp2 = blst_fp2 { fp: [fp_1, fp_2] };

    Ok(fp2)
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

    // Extract the four field elements (removing padding)
    let x_re = remove_padding(&input[..PADDED_FP_LENGTH])?;
    let x_im = remove_padding(&input[PADDED_FP_LENGTH..2 * PADDED_FP_LENGTH])?;
    let y_re = remove_padding(&input[2 * PADDED_FP_LENGTH..3 * PADDED_FP_LENGTH])?;
    let y_im = remove_padding(&input[3 * PADDED_FP_LENGTH..4 * PADDED_FP_LENGTH])?;

    // Convert bytes to point
    let point = decode_and_check_g2(x_re, x_im, y_re, y_im)?;

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

/// Checks whether or not the input represents a canonical field element, returning the field
/// element if successful.
fn fp_from_bendian(bytes: &[u8; 48]) -> Result<blst_fp, PrecompileError> {
    let mut fp = blst_fp::default();
    unsafe {
        // This performs the check for canonical field elements
        blst_fp_from_bendian(&mut fp, bytes.as_ptr());
    }
    Ok(fp)
}


/// Extracts a scalar value from a 32-byte input.
/// 
/// According to EIP-2537, the scalar input:
/// - Must be exactly 32 bytes
/// - Is interpreted as a big-endian integer
/// - Is not required to be less than the curve order
/// 
/// Returns a Result containing either the scalar value or a PrecompileError
pub(super) fn extract_scalar_input(input: &[u8]) -> Result<blst_scalar, PrecompileError> {
    // Check input length
    if input.len() != SCALAR_LENGTH {
        return Err(PrecompileError::IncorrectInputSize);
    }

    let mut scalar = blst_scalar::default();
    
    // Convert from big-endian bytes to scalar
    // SAFETY: Input length is checked above and scalar is properly initialized
    unsafe {
        blst_scalar_from_bendian(&mut scalar, input.as_ptr());
    }

    Ok(scalar)
}

/// Returns a `blst_p1_affine` from the provided byte slices, which represent the x and y
/// affine coordinates of the point.
///
/// If the x or y coordinate do not represent a canonical field element, an error is returned.
///
/// See [fp_from_bendian] for more information.
pub fn decode_and_check_g1(
    x_bytes: &[u8; 48],
    y_bytes: &[u8; 48],
) -> Result<blst_p1_affine, PrecompileError> {
    Ok(blst_p1_affine {
        x: fp_from_bendian(x_bytes)?,
        y: fp_from_bendian(y_bytes)?,
    })
}
/// Extracts a G1 point in Affine format from a 128 byte slice representation.
pub fn extract_g1_input(input: &[u8], subgroup_check: bool) -> Result<blst_p1_affine, PrecompileError> {
    if input.len() != G1_INPUT_LENGTH {
        return Err(PrecompileError::IncorrectInputSize);
    }

    // Split input and remove padding for x and y coordinates
    let x_bytes = remove_padding(&input[..PADDED_FP_LENGTH])?;
    let y_bytes = remove_padding(&input[PADDED_FP_LENGTH..G1_INPUT_LENGTH])?;
 
    let point = decode_and_check_g1(x_bytes, y_bytes)?;

    // Check if point is on curve (no subgroup check needed for addition)
    if subgroup_check {
        if unsafe { !blst_p1_affine_in_g1(&point) } {
            return Err(PrecompileError::InvalidInput);
        }
    }
    else{
        unsafe {
            if !blst_p1_affine_on_curve(&point) {
                return Err(PrecompileError::InvalidInput);
            }
        }
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
    out.into()
}
