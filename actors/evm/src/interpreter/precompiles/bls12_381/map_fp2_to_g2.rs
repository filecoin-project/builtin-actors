use crate::interpreter::{
    precompiles::{PrecompileContext, PrecompileError, PrecompileResult},
    System,
};
use fil_actors_runtime::runtime::Runtime;

use crate::interpreter::precompiles::bls_util::{
    PADDED_FP_LENGTH,
    PADDED_FP2_LENGTH,
    encode_g2_point,
    remove_padding,
};

use blst::{
    blst_fp2,
    blst_p2_affine,
    blst_map_to_g2,
};
/// BLS12_MAP_FP2_TO_G2 precompile
/// Implements mapping of field element to G2 point according to EIP-2537
pub fn bls12_map_fp2_to_g2<RT: Runtime>(
    _: &mut System<RT>,
    input: &[u8],
    _: PrecompileContext,
) -> PrecompileResult {
    // Check input length (should be 128 bytes)
    if input.len() != PADDED_FP2_LENGTH {
        return Err(PrecompileError::IncorrectInputSize);
    }

    // Split input into two 64-byte components
    let input_c0 = &input[..PADDED_FP_LENGTH];
    let input_c1 = &input[PADDED_FP_LENGTH..];

    // Remove padding from both components
    let c0_bytes = remove_padding(input_c0)?;
    let c1_bytes = remove_padding(input_c1)?;

    // Convert to Fp2 element
    let mut fp2 = blst_fp2::default();
    unsafe {
        // This performs the check for canonical field elements
        blst_fp2_from_bendian(&mut fp2, c0_bytes.as_ptr(), c1_bytes.as_ptr());
    }

    // Map the Fp2 element to a G2 point
    let mut p_aff = blst_p2_affine::default();
    unsafe {
        blst_map_to_g2(&mut p_aff, &fp2);
    }

    // Encode the result
    Ok(encode_g2_point(&p_aff))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::interpreter::System;
    use fil_actors_runtime::test_utils::MockRuntime;
    use hex_literal::hex;
}