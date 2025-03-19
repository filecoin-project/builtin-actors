use crate::interpreter::{
    precompiles::{PrecompileContext, PrecompileError, PrecompileResult},
    System,
};
use fil_actors_runtime::runtime::Runtime;

use crate::interpreter::precompiles::bls_util::{
    G1_INPUT_LENGTH,
    G2_INPUT_LENGTH,
    extract_g1_input,
    extract_g2_input,
};

use blst::{
    blst_miller_loop,
    blst_final_exp,
    blst_fp12,
    blst_p1_affine,
    blst_p2_affine,
};

/// Length of a single pairing input (G1 point + G2 point)
const PAIRING_INPUT_LENGTH: usize = G1_INPUT_LENGTH + G2_INPUT_LENGTH;

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
    let num_pairs = input_len / PAIRING_INPUT_LENGTH;
    
    // Collect pairs of points for the pairing check
    let mut pairs = Vec::with_capacity(num_pairs);
    
    for i in 0..num_pairs {
        let offset = i * PAIRING_INPUT_LENGTH;
        
        // Extract G1 point (with subgroup check)
        let g1_bytes = &input[offset..offset + G1_INPUT_LENGTH];
        let g1_point = extract_g1_input(g1_bytes, true)?;
        
        // Extract G2 point (with subgroup check)
        let g2_bytes = &input[offset + G1_INPUT_LENGTH..offset + PAIRING_INPUT_LENGTH];
        let g2_point = extract_g2_input(g2_bytes, true)?;
        
        pairs.push((g1_point, g2_point));
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

    let mut acc = blst_fp12::default();
    let mut tmp = blst_fp12::default();

    // Compute the product of pairings
    unsafe {
        // Initialize with first pairing
        blst_miller_loop(&mut acc, &pairs[0].1, &pairs[0].0);

        // Multiply by subsequent pairings
        for pair in pairs.iter().skip(1) {
            blst_miller_loop(&mut tmp, &pair.1, &pair.0);
            blst_fp12_mul(&mut acc, &acc, &tmp);
        }

        // Perform final exponentiation
        blst_final_exp(&mut acc, &acc);

        // Check if result is one
        blst_fp12_is_one(&acc)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::interpreter::System;
    use fil_actors_runtime::test_utils::MockRuntime;
    use hex_literal::hex;
}