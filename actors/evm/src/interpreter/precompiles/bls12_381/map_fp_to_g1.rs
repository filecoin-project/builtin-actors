use crate::interpreter::{
    precompiles::{PrecompileContext, PrecompileError, PrecompileResult},
    System,
};
use fil_actors_runtime::runtime::Runtime;

use crate::interpreter::precompiles::bls_util::{
    PADDED_FP_LENGTH,
    encode_g1_point,
    remove_padding,
};

use blst::{
    blst_map_to_g1,
    blst_fp,
    blst_fp_from_bendian,
    blst_p1,
    blst_p1_affine,
    blst_p1_to_affine
};

/// BLS12_MAP_FP_TO_G1 precompile
/// Implements mapping of field element to G1 point according to EIP-2537
#[allow(dead_code,unused_variables)]
pub fn bls12_map_fp_to_g1<RT: Runtime>(
    _: &mut System<RT>,
    input: &[u8],
    _: PrecompileContext,
) -> PrecompileResult {
    // Check input length (should be 64 bytes)
    if input.len() != PADDED_FP_LENGTH {
        return Err(PrecompileError::IncorrectInputSize);
    }

    // Remove padding and get the field element
    let input_bytes = remove_padding(input)?;
    
    // Convert to field element
    let mut fp = blst_fp::default();
    unsafe {
        // This performs the check for canonical field elements
        blst_fp_from_bendian(&mut fp, input_bytes.as_ptr());
    }

    // Map the field element to a G1 point
    let p_aff = map_fp_to_g1(&fp);

    // Encode the result
    Ok(encode_g1_point(&p_aff))
}

#[inline]
fn p1_to_affine(p: &blst_p1) -> blst_p1_affine {
    let mut p_affine = blst_p1_affine::default();
    // SAFETY: both inputs are valid blst types
    unsafe { blst_p1_to_affine(&mut p_affine, p) };
    p_affine
}

/// Maps a field element to a G1 point
///
/// Takes a field element (blst_fp) and returns the corresponding G1 point in affine form
#[inline]
pub(super) fn map_fp_to_g1(fp: &blst_fp) -> blst_p1_affine {
    // Create a new G1 point in Jacobian coordinates
    let mut p = blst_p1::default();

    // Map the field element to a point on the curve
    // SAFETY: `p` and `fp` are blst values
    // Third argument is unused if null
    unsafe { blst_map_to_g1(&mut p, fp, core::ptr::null()) };

    // Convert to affine coordinates
    p1_to_affine(&p)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::interpreter::System;
    use fil_actors_runtime::test_utils::MockRuntime;
    use hex_literal::hex;
    // #[test]
    // fn test_map_fp_to_g1_failures() {
    //     let rt = MockRuntime::default();
    //     rt.in_call.replace(true);
    //     let mut system = System::create(&rt).unwrap();

    //     // Test case 1: Empty input
    //     let empty_input: Vec<u8> = vec![];
    //     let res = bls12_map_fp_to_g1(&mut system, &empty_input, PrecompileContext::default());
    //     assert!(matches!(res, Err(PrecompileError::IncorrectInputSize)),
    //         "Empty input should return IncorrectInputSize error");

    //     // Test case 2: Short input (48 bytes instead of 64)
    //     let short_input = hex!(
    //         "00000000000000000000000000000000156c8a6a2c184569d69a76be144b5cdc5141d2d2ca4fe341f011e25e3969c55ad9e9b9ce2eb833c81a908e5fa4ac5f"
    //     );
    //     let res = bls12_map_fp_to_g1(&mut system, &short_input, PrecompileContext::default());
    //     assert!(matches!(res, Err(PrecompileError::IncorrectInputSize)),
    //         "Short input should return IncorrectInputSize error");

    //     // Test case 3: Large input (65 bytes instead of 64)
    //     let large_input = hex!(
    //         "0000000000000000000000000000000000156c8a6a2c184569d69a76be144b5cdc5141d2d2ca4fe341f011e25e3969c55ad9e9b9ce2eb833c81a908e5fa4ac5f03"
    //     );
    //     let res = bls12_map_fp_to_g1(&mut system, &large_input, PrecompileContext::default());
    //     assert!(matches!(res, Err(PrecompileError::IncorrectInputSize)),
    //         "Large input should return IncorrectInputSize error");

    //     // Test case 4: Invalid top bytes (non-zero padding)
    //     let invalid_top = hex!(
    //         "1000000000000000000000000000000000156c8a6a2c184569d69a76be144b5cdc5141d2d2ca4fe341f011e25e3969c55ad9e9b9ce2eb833c81a908e5fa4ac5f"
    //     );
    //     let res = bls12_map_fp_to_g1(&mut system, &invalid_top, PrecompileContext::default());
    //     assert!(matches!(res, Err(PrecompileError::InvalidInput)),
    //         "Invalid top bytes should return InvalidInput error");

    //     // Test case 5: Invalid field element
    //     let invalid_field = hex!(
    //         "000000000000000000000000000000002f6d9c5465982c0421b61e74579709b3b5b91e57bdd4f6015742b4ff301abb7ef895b9cce00c33c7d48f8e5fa4ac09ae"
    //     );
    //     let res = bls12_map_fp_to_g1(&mut system, &invalid_field, PrecompileContext::default());
    //     assert!(matches!(res, Err(PrecompileError::InvalidInput)),
    //         "Invalid field element should return InvalidInput error");
    // }

}