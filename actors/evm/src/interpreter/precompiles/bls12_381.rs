use super::PrecompileContext;
use super::PrecompileError;
use super::PrecompileResult;
use fil_actors_runtime::runtime::Runtime;
use crate::interpreter::System;

use blst::{
    blst_p1, blst_p1_add_or_double_affine, blst_p1_affine, blst_p1_from_affine, blst_p1_to_affine,
};

const G1_INPUT_LENGTH: usize = 128;
const G1_ADD_INPUT_LENGTH: usize = G1_INPUT_LENGTH * 2;

/// BLS12_G1ADD precompile
/// Implements G1 point addition according to EIP-2537
#[allow(dead_code, unused_variables)]
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

fn extract_g1_point(input: &[u8]) -> Result<blst_p1_affine, PrecompileError> {
    if input.len() != G1_INPUT_LENGTH {
        return Err(PrecompileError::IncorrectInputSize);
    }

    // Split input into x and y coordinates
    let _x_bytes = &input[0..64];
    let _y_bytes = &input[64..128];

    // TODO: Implement point deserialization from bytes to blst_p1_affine
    // This would involve:
    // 1. Converting bytes to Fp field elements
    // 2. Constructing the affine point
    // 3. Validating the point is on the curve
    
    unimplemented!("Point deserialization needs to be implemented")
}

fn encode_g1_point(point: &blst_p1_affine) -> Vec<u8> {
    // let mut output = Vec::with_capacity(G1_INPUT_LENGTH);
    
    // TODO: Implement point serialization from blst_p1_affine to bytes
    // This would involve:
    // 1. Extracting x and y coordinates
    // 2. Converting field elements to bytes
    // 3. Concatenating the results
    
    unimplemented!("Point serialization needs to be implemented")
}