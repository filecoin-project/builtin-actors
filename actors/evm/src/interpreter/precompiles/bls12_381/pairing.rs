use crate::interpreter::{
    precompiles::{PrecompileContext, PrecompileError, PrecompileResult},
    System,
};
use fil_actors_runtime::runtime::Runtime;

use crate::interpreter::precompiles::bls_util::{
    PAIRING_INPUT_LENGTH,
    PADDED_G1_LENGTH,
    PADDED_G2_LENGTH,
    extract_g1_input,
    extract_g2_input,
};

use blst::{
    blst_miller_loop,
    blst_final_exp,
    blst_fp12,
    blst_fp12_mul,
    blst_fp12_is_one,
    blst_p1_affine,
    blst_p2_affine,
};

/// BLS12_PAIRING precompile
/// Implements BLS12-381 pairing check according to EIP-2537
pub fn bls12_pairing<RT: Runtime>(
    _: &mut System<RT>,
    input: &[u8],
    _: PrecompileContext,
) -> PrecompileResult {
    let input_len = input.len();
    
    // Input length must be non-zero and multiple of the pairing input length
    if input_len == 0 || input_len % PAIRING_INPUT_LENGTH != 0 {
        return Err(PrecompileError::IncorrectInputSize);
    }

    // Calculate number of pairs
    let k = input_len / PAIRING_INPUT_LENGTH;
    
    // Collect pairs of points for the pairing check
    let mut pairs = Vec::with_capacity(k);
    
    for i in 0..k {
        let encoded_g1_element =
            &input[i * PAIRING_INPUT_LENGTH..i * PAIRING_INPUT_LENGTH + PADDED_G1_LENGTH];
        let encoded_g2_element = &input[i * PAIRING_INPUT_LENGTH + PADDED_G1_LENGTH
            ..i * PAIRING_INPUT_LENGTH + PADDED_G1_LENGTH + PADDED_G2_LENGTH];

        // If either the G1 or G2 element is the encoded representation
        // of the point at infinity, then these two points are no-ops
        // in the pairing computation.
        //
        // Note: we do not skip the validation of these two elements even if
        // one of them is the point at infinity because we could have G1 be
        // the point at infinity and G2 be an invalid element or vice versa.
        // In that case, the precompile should error because one of the elements
        // was invalid.
        let g1_is_zero = encoded_g1_element.iter().all(|i| *i == 0);
        let g2_is_zero = encoded_g2_element.iter().all(|i| *i == 0);

        // NB: Scalar multiplications, MSMs and pairings MUST perform a subgroup check.
        // extract_g1_input and extract_g2_input perform the necessary checks
        let p1_aff = extract_g1_input(encoded_g1_element, true)?;
        let p2_aff = extract_g2_input(encoded_g2_element, true)?;

        if !g1_is_zero & !g2_is_zero {
            pairs.push((p1_aff, p2_aff));
        }
    }

    // Perform the pairing check
    let result = pairing_check(&pairs);
    
    // Return 32 bytes: 31 zero bytes followed by 1 for success, 0 for failure
    let mut output = vec![0u8; 32];
    output[31] = if result { 1 } else { 0 };
    
    Ok(output)
}

/// Helper function to perform the pairing check
fn pairing_check(pairs: &[(blst_p1_affine, blst_p2_affine)]) -> bool {
    if pairs.is_empty() {
        return true;
    }

    // Compute the miller loop for the first pair
    let (first_g1, first_g2) = &pairs[0];
    let mut acc = compute_miller_loop(first_g1, first_g2);

    // For the remaining pairs, compute miller loop and multiply with the accumulated result
    for (g1, g2) in pairs.iter().skip(1) {
        let ml = compute_miller_loop(g1, g2);
        acc = multiply_fp12(&acc, &ml);
    }

    // Apply final exponentiation and check if result is 1
    let final_result = final_exp(&acc);

    // Check if the result is one (identity element)
    is_fp12_one(&final_result)
}

/// Computes a single miller loop for a given G1, G2 pair
#[inline]
fn compute_miller_loop(g1: &blst_p1_affine, g2: &blst_p2_affine) -> blst_fp12 {
    let mut result = blst_fp12::default();

    // SAFETY: All arguments are valid blst types
    unsafe { blst_miller_loop(&mut result, g2, g1) }

    result
}
// multiply_fp12 multiplies two fp12 elements
#[inline]
fn multiply_fp12(a: &blst_fp12, b: &blst_fp12) -> blst_fp12 {
    let mut result = blst_fp12::default();

    // SAFETY: All arguments are valid blst types
    unsafe { blst_fp12_mul(&mut result, a, b) }

    result
}

/// final_exp computes the final exponentiation on an fp12 element
#[inline]
fn final_exp(f: &blst_fp12) -> blst_fp12 {
    let mut result = blst_fp12::default();

    // SAFETY: All arguments are valid blst types
    unsafe { blst_final_exp(&mut result, f) }

    result
}

/// is_fp12_one checks if an fp12 element equals
/// multiplicative identity element, one
#[inline]
fn is_fp12_one(f: &blst_fp12) -> bool {
    // SAFETY: argument is a valid blst type
    unsafe { blst_fp12_is_one(f) }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::interpreter::System;
    use fil_actors_runtime::test_utils::MockRuntime;
    use hex_literal::hex;
}