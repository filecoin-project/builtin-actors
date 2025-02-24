use super::PrecompileContext;
use super::PrecompileError;
use super::PrecompileResult;
use fil_actors_runtime::runtime::Runtime;
use crate::interpreter::System;

use blst::{
    blst_p1, blst_p1_add_or_double_affine, blst_p1_affine, blst_p1_from_affine, blst_p1_to_affine, blst_fp, blst_p1_affine_on_curve
};

const G1_INPUT_LENGTH: usize = 128;
const G1_ADD_INPUT_LENGTH: usize = G1_INPUT_LENGTH * 2;
const G1_OUTPUT_LENGTH: usize = 128;
pub const PADDED_FP_LENGTH: usize = 64;

/// Encodes a single finite field element into byte slice with padding.
fn fp_to_bytes(out: &mut [u8], x: &blst_fp) {
    unsafe {
        let x_bytes: [u8; 48] = std::mem::transmute(*x);
        out.copy_from_slice(&x_bytes);
    }
}

/// Checks whether or not the input represents a canonical field element, returning the field
/// element if successful.
fn fp_from_bendian(bytes: &[u8; 48]) -> Result<blst_fp, PrecompileError> {
    let mut fp = blst_fp::default();
    unsafe {
        let fp_bytes: &mut [u8; 48] = std::mem::transmute(&mut fp);
        fp_bytes.copy_from_slice(bytes);
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
#[allow(dead_code,unused_variables)]
pub(super) fn bls12_g1_add<RT: Runtime>(
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

    // Split input into x and y coordinates
    let x_bytes: &[u8; 48] = input[..48].try_into()
        .map_err(|_| PrecompileError::IncorrectInputSize)?;
    let y_bytes: &[u8; 48] = input[48..96].try_into()
        .map_err(|_| PrecompileError::IncorrectInputSize)?;

    let point = decode_and_check_g1(x_bytes, y_bytes)?;

    // Check if point is on curve (no subgroup check needed for addition)
    unsafe {
        if !blst_p1_affine_on_curve(&point) {
            return Err(PrecompileError::InvalidInput);
        }
    }

    Ok(point)
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