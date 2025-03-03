use super::PrecompileContext;
use super::PrecompileError;
use super::PrecompileResult;
use fil_actors_runtime::runtime::Runtime;
use crate::interpreter::System;

use blst::{
    blst_p1, blst_p1_add_or_double_affine, blst_p1_affine, blst_p1_from_affine, blst_p1_to_affine, blst_fp, blst_p1_affine_on_curve, blst_fp_from_bendian, blst_bendian_from_fp
};

const G1_INPUT_LENGTH: usize = 128;
const G1_ADD_INPUT_LENGTH: usize = G1_INPUT_LENGTH * 2;
const G1_OUTPUT_LENGTH: usize = 128;
/// Finite field element padded input length.
pub const PADDED_FP_LENGTH: usize = 64;
/// Input elements padding length.
pub const PADDING_LENGTH: usize = 16;

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
    let a_aff = extract_g1_point(a_bytes)?;
    let b_aff = extract_g1_point(b_bytes)?;

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
fn extract_g1_point(input: &[u8]) -> Result<blst_p1_affine, PrecompileError> {
    if input.len() != G1_INPUT_LENGTH {
        return Err(PrecompileError::IncorrectInputSize);
    }

    // Split input and remove padding for x and y coordinates
    let x_bytes = remove_padding(&input[..PADDED_FP_LENGTH])?;
    let y_bytes = remove_padding(&input[PADDED_FP_LENGTH..G1_INPUT_LENGTH])?;
 
    let point = decode_and_check_g1(x_bytes, y_bytes)?;

    // Check if point is on curve (no subgroup check needed for addition)
    unsafe {
        if !blst_p1_affine_on_curve(&point) {
            return Err(PrecompileError::InvalidInput);
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

/// BLS12_G1MSM precompile
/// Implements G1 multi-scalar multiplication according to EIP-2537
#[allow(dead_code,unused_variables)]
pub(super) fn bls12_g1msm<RT: Runtime>(
    _: &mut System<RT>,
    input: &[u8],
    _: PrecompileContext,
) -> PrecompileResult {
    Err(PrecompileError::CallForbidden)
}

/// BLS12_G2ADD precompile
/// Implements G2 point addition according to EIP-2537
#[allow(dead_code,unused_variables)]
pub(super) fn bls12_g2add<RT: Runtime>(
    _: &mut System<RT>,
    input: &[u8],
    _: PrecompileContext,
) -> PrecompileResult {
    Err(PrecompileError::CallForbidden)
}

/// BLS12_G2MSM precompile
/// Implements G2 multi-scalar multiplication according to EIP-2537
#[allow(dead_code,unused_variables)]
pub(super) fn bls12_g2msm<RT: Runtime>(
    _: &mut System<RT>,
    input: &[u8],
    _: PrecompileContext,
) -> PrecompileResult {
    Err(PrecompileError::CallForbidden)
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
        let input = hex::decode(
            "\
            1111111111111111111111111111111111111111111111111111111111111111\
            1111111111111111111111111111111111111111111111111111111111111111\
            2222222222222222222222222222222222222222222222222222222222222222\
            2222222222222222222222222222222222222222222222222222222222222222\
            3333333333333333333333333333333333333333333333333333333333333333\
            3333333333333333333333333333333333333333333333333333333333333333\
            4444444444444444444444444444444444444444444444444444444444444444\
            4444444444444444444444444444444444444444444444444444444444444444"
        ).unwrap();

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
}