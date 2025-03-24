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
    check_canonical_fp2,
};

use blst::{
    blst_fp2,
    blst_p2,
    blst_p2_affine,
    blst_p2_to_affine,
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
    let input_c1 = &input[PADDED_FP_LENGTH..PADDED_FP2_LENGTH];

    // Remove padding from both components
    let c0_bytes = remove_padding(input_c0)?;
    let c1_bytes = remove_padding(input_c1)?;

    let fp2 = check_canonical_fp2(c0_bytes, c1_bytes)?;
    // Map the Fp2 element to a G2 point
    let p_aff = map_fp2_to_g2(&fp2);

    // Encode the result
    Ok(encode_g2_point(&p_aff))
}
/// Maps a field element to a G2 point
///
/// Takes a field element (blst_fp2) and returns the corresponding G2 point in affine form
#[inline]
pub(super) fn map_fp2_to_g2(fp2: &blst_fp2) -> blst_p2_affine {
    // Create a new G2 point in Jacobian coordinates
    let mut p = blst_p2::default();

    // Map the field element to a point on the curve
    // SAFETY: `p` and `fp2` are blst values
    // Third argument is unused if null
    unsafe { blst_map_to_g2(&mut p, fp2, core::ptr::null()) };

    // Convert to affine coordinates
    p2_to_affine(&p)
}
#[inline]
fn p2_to_affine(p: &blst_p2) -> blst_p2_affine {
    let mut p_affine = blst_p2_affine::default();
    // SAFETY: both inputs are valid blst types
    unsafe { blst_p2_to_affine(&mut p_affine, p) };
    p_affine
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::interpreter::System;
    use fil_actors_runtime::test_utils::MockRuntime;
    use hex_literal::hex;
}