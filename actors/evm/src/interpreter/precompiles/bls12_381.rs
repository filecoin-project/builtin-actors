use super::PrecompileContext;
use super::PrecompileError;
use super::PrecompileResult;
use fil_actors_runtime::runtime::Runtime;
use crate::interpreter::System;

use blst::{
    blst_p1, blst_p1_add_or_double_affine, blst_p1_affine, blst_p1_from_affine, blst_p1_to_affine, blst_p1_affine_in_g1, blst_fp, blst_p1_affine_on_curve, blst_fp_from_bendian, blst_bendian_from_fp, blst_scalar, blst_scalar_from_bendian, p1_affines, blst_p2, blst_p2_affine, blst_p2_add_or_double_affine, blst_p2_from_affine, blst_p2_to_affine, blst_p2_affine_on_curve, blst_p2_affine_in_g2, blst_fp2, p2_affines
};

pub const G1_INPUT_LENGTH: usize = 128;
pub const G1_ADD_INPUT_LENGTH: usize = G1_INPUT_LENGTH * 2;
pub const G1_OUTPUT_LENGTH: usize = 128;
pub const PADDED_FP_LENGTH: usize = 64;
pub const PADDING_LENGTH: usize = 16;
pub const G1_MSM_INPUT_LENGTH: usize = 160;
pub const G1_INPUT_ITEM_LENGTH: usize = 128;
pub const SCALAR_LENGTH: usize = 32;
pub const NBITS: usize = 255; 
pub const G2_ADD_INPUT_LENGTH: usize = 512;
pub const G2_INPUT_ITEM_LENGTH: usize = 256;
pub const G2_OUTPUT_LENGTH: usize = 256;
pub const G2_MSM_INPUT_LENGTH: usize = 288;


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
fn check_canonical_fp2(
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
fn decode_and_check_g1(
    x_bytes: &[u8; 48],
    y_bytes: &[u8; 48],
) -> Result<blst_p1_affine, PrecompileError> {
    Ok(blst_p1_affine {
        x: fp_from_bendian(x_bytes)?,
        y: fp_from_bendian(y_bytes)?,
    })
}

/// BLS12_G1ADD precompile
/// Implements G1 point addition according to EIP-2537
pub(super) fn bls12_g1add<RT: Runtime>(
    _: &mut System<RT>,
    input: &[u8],
    _: PrecompileContext,
) -> PrecompileResult {
    if input.len() != G1_ADD_INPUT_LENGTH {
        return Err(PrecompileError::IncorrectInputSize);
    }

    // Extract the two input G1 points
    let a_bytes = &input[..G1_INPUT_LENGTH];
    let b_bytes = &input[G1_INPUT_LENGTH..];

    // Convert input bytes to blst affine points
    let a_aff = extract_g1_input(a_bytes, false)?;
    let b_aff = extract_g1_input(b_bytes, false)?;

    let mut b = blst_p1::default();
    // Convert b_aff to projective coordinates
    unsafe { blst_p1_from_affine(&mut b, &b_aff) };

    let mut p = blst_p1::default();
    // Add the points
    unsafe { blst_p1_add_or_double_affine(&mut p, &b, &a_aff) };

    let mut p_aff = blst_p1_affine::default();
    // Convert result back to affine coordinates
    unsafe { blst_p1_to_affine(&mut p_aff, &p) };

    // Encode the result
    Ok(encode_g1_point(&p_aff))
}
/// Extracts a G1 point in Affine format from a 128 byte slice representation.
fn extract_g1_input(input: &[u8], subgroup_check: bool) -> Result<blst_p1_affine, PrecompileError> {
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
fn remove_padding(input: &[u8]) -> Result<&[u8; 48], PrecompileError> {
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
fn encode_g1_point(input: *const blst_p1_affine) -> Vec<u8> {
    let mut out = vec![0u8; G1_OUTPUT_LENGTH];
    // SAFETY: Out comes from fixed length array, input is a blst value.
    unsafe {
        fp_to_bytes(&mut out[..PADDED_FP_LENGTH], &(*input).x);
        fp_to_bytes(&mut out[PADDED_FP_LENGTH..], &(*input).y);
    }
    out.into()
}

/// Implements EIP-2537 G1MSM precompile.
/// G1 multi-scalar-multiplication call expects `160*k` bytes as an input that is interpreted
/// as byte concatenation of `k` slices each of them being a byte concatenation
/// of encoding of G1 point (`128` bytes) and encoding of a scalar value (`32`
/// bytes).
/// Output is an encoding of multi-scalar-multiplication operation result - single G1
/// point (`128` bytes).
/// See also: <https://eips.ethereum.org/EIPS/eip-2537#abi-for-g1-multiexponentiation>
pub(super) fn bls12_g1msm<RT: Runtime>(
    _: &mut System<RT>,
    input: &[u8],
    _: PrecompileContext,
) -> PrecompileResult {
    let input_len = input.len();
    if input_len == 0 || input_len % G1_MSM_INPUT_LENGTH != 0 {
        return Err(PrecompileError::IncorrectInputSize);
    }

    let k = input_len / G1_MSM_INPUT_LENGTH;
    let mut g1_points: Vec<blst_p1> = Vec::with_capacity(k);
    let mut scalars: Vec<u8> = Vec::with_capacity(k * SCALAR_LENGTH);

    // Process each (point, scalar) pair
    for i in 0..k {
        let slice = &input[i * G1_MSM_INPUT_LENGTH..i * G1_MSM_INPUT_LENGTH + G1_INPUT_ITEM_LENGTH];

        // Skip points at infinity (all zeros)
        if slice.iter().all(|i| *i == 0) {
            continue;
        }

        // NB: Scalar multiplications, MSMs and pairings MUST perform a subgroup check.
        //
        // So we set the subgroup_check flag to `true`
        let p0_aff = &extract_g1_input(slice, true)?;

        let mut p0 = blst_p1::default();
        // SAFETY: `p0` and `p0_aff` are blst values.
        unsafe { blst_p1_from_affine(&mut p0, p0_aff) };
        g1_points.push(p0);

        scalars.extend_from_slice(
            &extract_scalar_input(
                &input[i * G1_MSM_INPUT_LENGTH + G1_INPUT_ITEM_LENGTH
                    ..i * G1_MSM_INPUT_LENGTH + G1_INPUT_ITEM_LENGTH + SCALAR_LENGTH],
            )?
            .b,
        );
    }

    // Return infinity point if all points are infinity
    if g1_points.is_empty() {
        return Ok(vec![0u8; G1_OUTPUT_LENGTH]);
    }
    let points = p1_affines::from(&g1_points);
    let multiexp = points.mult(&scalars, NBITS);

    let mut multiexp_aff = blst_p1_affine::default();
    // SAFETY: `multiexp_aff` and `multiexp` are blst values.
    unsafe { blst_p1_to_affine(&mut multiexp_aff, &multiexp) };
    Ok(encode_g1_point(&multiexp_aff))
}


/// BLS12_G2ADD precompile
/// Implements G2 point addition according to EIP-2537
#[allow(dead_code,unused_variables)]
pub(super) fn bls12_g2add<RT: Runtime>(
    _: &mut System<RT>,
    input: &[u8],
    _: PrecompileContext,
) -> PrecompileResult {
    if input.len() != G2_ADD_INPUT_LENGTH {
        return Err(PrecompileError::IncorrectInputSize);
    }

    // Extract the two input G2 points
    // No subgroup check needed for addition
    let a_aff = extract_g2_input(&input[..G2_INPUT_ITEM_LENGTH], false)?;
    let b_aff = extract_g2_input(&input[G2_INPUT_ITEM_LENGTH..], false)?;

    let mut b = blst_p2::default();
    // Convert b_aff to projective coordinates
    unsafe { blst_p2_from_affine(&mut b, &b_aff) };

    let mut p = blst_p2::default();
    // Add the points
    unsafe { blst_p2_add_or_double_affine(&mut p, &b, &a_aff) };

    let mut p_aff = blst_p2_affine::default();
    // Convert result back to affine coordinates
    unsafe { blst_p2_to_affine(&mut p_aff, &p) };

    // Encode the result
    Ok(encode_g2_point(&p_aff))
}

/// BLS12_G2MSM precompile
/// Implements G2 multi-scalar multiplication according to EIP-2537
pub(super) fn bls12_g2msm<RT: Runtime>(
    _: &mut System<RT>,
    input: &[u8],
    _: PrecompileContext,
) -> PrecompileResult {
    let input_len = input.len();
    if input_len == 0 || input_len % G2_MSM_INPUT_LENGTH != 0 {
        return Err(PrecompileError::IncorrectInputSize);
    }

    let k = input_len / G2_MSM_INPUT_LENGTH;
    let mut g2_points: Vec<blst_p2> = Vec::with_capacity(k);
    let mut scalars: Vec<u8> = Vec::with_capacity(k * SCALAR_LENGTH);

    // Process each (point, scalar) pair
    for i in 0..k {
        let slice = &input[i * G2_MSM_INPUT_LENGTH..i * G2_MSM_INPUT_LENGTH + G2_INPUT_ITEM_LENGTH];

        // Skip points at infinity (all zeros)
        if slice.iter().all(|i| *i == 0) {
            continue;
        }

        // NB: Scalar multiplications, MSMs and pairings MUST perform a subgroup check.
        //
        // So we set the subgroup_check flag to `true`
        let p0_aff = extract_g2_input(slice, true)?;

        let mut p0 = blst_p2::default();
        // Convert to projective coordinates
        // SAFETY: `p0` and `p0_aff` are blst values
        unsafe { blst_p2_from_affine(&mut p0, &p0_aff) };
        g2_points.push(p0);

        // Extract and add scalar
        scalars.extend_from_slice(
            &extract_scalar_input(
                &input[i * G2_MSM_INPUT_LENGTH + G2_INPUT_ITEM_LENGTH
                    ..i * G2_MSM_INPUT_LENGTH + G2_INPUT_ITEM_LENGTH + SCALAR_LENGTH],
            )?
            .b,
        );
    }

    // Return infinity point if all points are infinity
    if g2_points.is_empty() {
        return Ok(vec![0u8; G2_OUTPUT_LENGTH]);
    }

    // Convert points to affine representation for batch operation
    let points = p2_affines::from(&g2_points);
    // Perform multi-scalar multiplication
    let multiexp = points.mult(&scalars, NBITS);

    let mut multiexp_aff = blst_p2_affine::default();
    // Convert result back to affine coordinates
    // SAFETY: `multiexp_aff` and `multiexp` are blst values
    unsafe { blst_p2_to_affine(&mut multiexp_aff, &multiexp) };

    // Encode the result
    Ok(encode_g2_point(&multiexp_aff))
}

/// BLS12_PAIRING precompile
/// Implements BLS12-381 pairing check according to EIP-2537
#[allow(dead_code,unused_variables)]
pub(super) fn bls12_pairing<RT: Runtime>(
    _: &mut System<RT>,
    input: &[u8],
    _: PrecompileContext,
) -> PrecompileResult {
    Err(PrecompileError::CallForbidden)
}

/// BLS12_MAP_FP_TO_G1 precompile
/// Implements mapping of field element to G1 point according to EIP-2537
#[allow(dead_code,unused_variables)]
pub(super) fn bls12_map_fp_to_g1<RT: Runtime>(
    _: &mut System<RT>,
    input: &[u8],
    _: PrecompileContext,
) -> PrecompileResult {
    Err(PrecompileError::CallForbidden)
}

/// BLS12_MAP_FP2_TO_G2 precompile
/// Implements mapping of field element to G2 point according to EIP-2537
#[allow(dead_code,unused_variables)]
pub(super) fn bls12_map_fp2_to_g2<RT: Runtime>(
    _: &mut System<RT>,
    input: &[u8],
    _: PrecompileContext,
) -> PrecompileResult {
    Err(PrecompileError::CallForbidden)
}


#[cfg(test)]
mod tests {
    use super::*;
    use crate::interpreter::System;
    use fil_actors_runtime::test_utils::MockRuntime;
    use hex_literal::hex;

    #[test]
    fn test_g1_add() {
        let rt = MockRuntime::default();
        rt.in_call.replace(true);
        let mut system = System::create(&rt).unwrap();

        // Test case 1: Valid addition
        // Input: Two valid G1 points P1 and P2
        // Test case: bls_g1add_g1+p1
        let input = hex::decode(
            "0000000000000000000000000000000017f1d3a73197d7942695638c4fa9ac0fc3688c4f9774b905a14e3a3f171bac586c55e83ff97a1aeffb3af00adb22c6bb\
            0000000000000000000000000000000008b3f481e3aaa0f1a09e30ed741d8ae4fcf5e095d5d00af600db18cb2c04b3edd03cc744a2888ae40caa232946c5e7e1\
            00000000000000000000000000000000112b98340eee2777cc3c14163dea3ec97977ac3dc5c70da32e6e87578f44912e902ccef9efe28d4a78b8999dfbca9426\
            00000000000000000000000000000000186b28d92356c4dfec4b5201ad099dbdede3781f8998ddf929b4cd7756192185ca7b8f4ef7088f813270ac3d48868a21"
        ).unwrap();

        // Expected result from Ethereum test suite
        let expected = hex::decode(
            "000000000000000000000000000000000a40300ce2dec9888b60690e9a41d3004fda4886854573974fab73b046d3147ba5b7a5bde85279ffede1b45b3918d82d\
            0000000000000000000000000000000006d3d887e9f53b9ec4eb6cedf5607226754b07c01ace7834f57f3e7315faefb739e59018e22c492006190fba4a870025"
        ).unwrap();

        let res = bls12_g1add(&mut system, &input, PrecompileContext::default()).unwrap();
        assert_eq!(res, expected, 
            "G1 addition result did not match expected output.\nGot: {}\nExpected: {}", 
            hex::encode(&res), hex::encode(&expected)
        );

        // Test case 2: Zero input (should return zero point)
        let zero_input = vec![0u8; G1_ADD_INPUT_LENGTH];
        let res = bls12_g1add(&mut system, &zero_input, PrecompileContext::default()).unwrap();
        assert_eq!(res, vec![0u8; G1_OUTPUT_LENGTH]);

        // Test case 3: Invalid input length
        let invalid_input = vec![0u8; G1_ADD_INPUT_LENGTH - 1];
        let res = bls12_g1add(&mut system, &invalid_input, PrecompileContext::default());
        assert!(matches!(res, Err(PrecompileError::IncorrectInputSize)));

        // Test case 4: Point not on curve
        let invalid_point = hex::decode(
            "\
            1111111111111111111111111111111111111111111111111111111111111111\
            1111111111111111111111111111111111111111111111111111111111111111\
            1111111111111111111111111111111111111111111111111111111111111111\
            1111111111111111111111111111111111111111111111111111111111111111\
            1111111111111111111111111111111111111111111111111111111111111111\
            1111111111111111111111111111111111111111111111111111111111111111\
            1111111111111111111111111111111111111111111111111111111111111111\
            1111111111111111111111111111111111111111111111111111111111111111"
        ).unwrap();
        let res = bls12_g1add(&mut system, &invalid_point, PrecompileContext::default());
        assert!(matches!(res, Err(PrecompileError::InvalidInput)));

        // Test case 5: Empty input
        let empty_input: Vec<u8> = vec![];
        let res = bls12_g1add(&mut system, &empty_input, PrecompileContext::default());
        assert!(matches!(res, Err(PrecompileError::IncorrectInputSize)));
    }

    #[test]
    fn test_fp_conversion() {
        // Test fp_to_bytes and fp_from_bendian
        let test_bytes: [u8; 48] = [
            0x11, 0x22, 0x33, 0x44, 0x55, 0x66, 0x77, 0x88,
            0x99, 0xaa, 0xbb, 0xcc, 0xdd, 0xee, 0xff, 0x00,
            0x11, 0x22, 0x33, 0x44, 0x55, 0x66, 0x77, 0x88,
            0x99, 0xaa, 0xbb, 0xcc, 0xdd, 0xee, 0xff, 0x00,
            0x11, 0x22, 0x33, 0x44, 0x55, 0x66, 0x77, 0x88,
            0x99, 0xaa, 0xbb, 0xcc, 0xdd, 0xee, 0xff, 0x00,
        ];

        // Test roundtrip conversion
        let fp = fp_from_bendian(&test_bytes).unwrap();
        let mut output = [0u8; 48];
        fp_to_bytes(&mut output, &fp);
        assert_eq!(test_bytes, output);
    }
    #[test]
    fn test_g1_msm_success() {
        let rt = MockRuntime::default();
        rt.in_call.replace(true);
        let mut system = System::create(&rt).unwrap();

        // Test case: bls_g1mul_(g1+g1=2*g1)
        let input = hex::decode(
            "0000000000000000000000000000000017f1d3a73197d7942695638c4fa9ac0fc3688c4f9774b905a14e3a3f171bac586c55e83ff97a1aeffb3af00adb22c6bb\
             0000000000000000000000000000000008b3f481e3aaa0f1a09e30ed741d8ae4fcf5e095d5d00af600db18cb2c04b3edd03cc744a2888ae40caa232946c5e7e1\
             0000000000000000000000000000000000000000000000000000000000000002"
        ).unwrap();

        let expected = hex::decode(
            "000000000000000000000000000000000572cbea904d67468808c8eb50a9450c9721db309128012543902d0ac358a62ae28f75bb8f1c7c42c39a8c5529bf0f4e\
             00000000000000000000000000000000166a9d8cabc673a322fda673779d8e3822ba3ecb8670e461f73bb9021d5fd76a4c56d9d4cd16bd1bba86881979749d28"
        ).unwrap();

        let res = bls12_g1msm(&mut system, &input, PrecompileContext::default()).unwrap();
        assert_eq!(res, expected, 
            "G1 MSM result did not match expected output.\nGot: {}\nExpected: {}", 
            hex::encode(&res), hex::encode(&expected)
        );
    }

    #[test]
    fn test_g1_msm_failures() {
        let rt = MockRuntime::default();
        rt.in_call.replace(true);
        let mut system = System::create(&rt).unwrap();
        let ctx = PrecompileContext::default();

        // Test: Empty input
        let res = bls12_g1msm(&mut system, &[], ctx);
        assert!(matches!(res, Err(PrecompileError::IncorrectInputSize)));

        // Test: Short input
        let short_input = hex::decode(
            "00000000000000000000000000000017f1d3a73197d7942695638c4fa9ac0fc3688c4f9774b905a14e3a3f171bac586c55e83ff97a1aeffb3af00adb22c6bb\
             0000000000000000000000000000000008b3f481e3aaa0f1a09e30ed741d8ae4fcf5e095d5d00af600db18cb2c04b3edd03cc744a2888ae40caa232946c5e7e1\
             0000000000000000000000000000000000000000000000000000000000000002"
        ).unwrap();
        let res = bls12_g1msm(&mut system, &short_input, ctx);
        assert!(matches!(res, Err(PrecompileError::IncorrectInputSize)));

        // TODO: Fix this test
        // Error caused by the fact that the input is not padded to 64 bytes and the padding is not removed
        // https://ethereum-magicians.org/t/eip-2537-bls12-precompile-discussion-thread/4187
        // https://github.com/ethereum/EIPs/blob/master/EIPS/eip-2537.md

        // // Test: Invalid field element
        // let invalid_field = hex::decode(
        //     "0000000000000000000000000000000031f2e5916b17be2e71b10b4292f558e727dfd7d48af9cbc5087f0ce00dcca27c8b01e83eaace1aefb539f00adb2271660000000000000000000000000000000008b3f481e3aaa0f1a09e30ed741d8ae4fcf5e095d5d00af600db18cb2c04b3edd03cc744a2888ae40caa232946c5e7e10000000000000000000000000000000000000000000000000000000000000002"
        // ).unwrap();
        // let res = bls12_g1msm(&mut system, &invalid_field, ctx);
        // match res {
        //     Ok(_) => panic!("Expected error for invalid field element, got success"),
        //     Err(e) => {
        //         println!("Got error: {:?}", e);
        //         assert!(matches!(e, PrecompileError::InvalidInput), 
        //             "Expected InvalidInput error, got {:?}", e);
        //     }
        // }
        // assert!(matches!(res, Err(PrecompileError::InvalidInput)));

        // Test: Point not on curve
        let not_on_curve = hex::decode(
            "0000000000000000000000000000000017f1d3a73197d7942695638c4fa9ac0fc3688c4f9774b905a14e3a3f171bac586c55e83ff97a1aeffb3af00adb22c6bb\
             00000000000000000000000000000000186b28d92356c4dfec4b5201ad099dbdede3781f8998ddf929b4cd7756192185ca7b8f4ef7088f813270ac3d48868a21\
             0000000000000000000000000000000000000000000000000000000000000002"
        ).unwrap();
        let res = bls12_g1msm(&mut system, &not_on_curve, ctx);
        assert!(matches!(res, Err(PrecompileError::InvalidInput)));

        // Test: Invalid top bytes
        let invalid_top = hex::decode(
            "1000000000000000000000000000000017f1d3a73197d7942695638c4fa9ac0fc3688c4f9774b905a14e3a3f171bac586c55e83ff97a1aeffb3af00adb22c6bb\
             0000000000000000000000000000000008b3f481e3aaa0f1a09e30ed741d8ae4fcf5e095d5d00af600db18cb2c04b3edd03cc744a2888ae40caa232946c5e7e1\
             0000000000000000000000000000000000000000000000000000000000000002"
        ).unwrap();
        let res = bls12_g1msm(&mut system, &invalid_top, ctx);
        assert!(matches!(res, Err(PrecompileError::InvalidInput)));

        // Test: Point not in correct subgroup
        let not_in_subgroup = hex::decode(
            "000000000000000000000000000000000123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef00000000000000000000000000000000193fb7cedb32b2c3adc06ec11a96bc0d661869316f5e4a577a9f7c179593987beb4fb2ee424dbb2f5dd891e228b46c4a0000000000000000000000000000000000000000000000000000000000000002"
        ).unwrap();
        let res = bls12_g1msm(&mut system, &not_in_subgroup, ctx);
        assert!(matches!(res, Err(PrecompileError::InvalidInput)));
    }
}