use crate::interpreter::{
    System,
    precompiles::{PrecompileContext, PrecompileError, PrecompileResult},
};
use fil_actors_runtime::runtime::Runtime;

use crate::interpreter::precompiles::bls_util::{
    PADDED_FP_LENGTH, encode_g1_point, p1_to_affine, read_fp, remove_padding,
};

use blst::{blst_fp, blst_map_to_g1, blst_p1, blst_p1_affine};

/// **BLS12_MAP_FP_TO_G1 Precompile**
///
/// Implements mapping of a field element in Fp to a G1 point according to
/// [EIP-2537](https://eips.ethereum.org/EIPS/eip-2537#abi-for-g1-multiexponentiation).
///
/// The input must be exactly **64 bytes** (a padded Fp element). The output is the 128‑byte encoding
/// of the resulting G1 point.
#[allow(dead_code, unused_variables)]
pub fn bls12_map_fp_to_g1<RT: Runtime>(
    _: &mut System<RT>,
    input: &[u8],
    _: PrecompileContext,
) -> PrecompileResult {
    // Ensure the input is exactly PADDED_FP_LENGTH (64 bytes).
    if input.len() != PADDED_FP_LENGTH {
        return Err(PrecompileError::IncorrectInputSize);
    }

    // Remove padding and obtain the Fp element.
    let unpadded = remove_padding(input)?;
    let fp = read_fp(unpadded)?;

    // Map the Fp element to a G1 point.
    let p_aff = map_fp_to_g1(&fp);

    // Encode the resulting G1 point and return.
    Ok(encode_g1_point(&p_aff))
}

/// Maps an Fp field element to a G1 point (affine form).
///
/// Note: While this function contains an unsafe block for BLST operations,
/// the function itself is safe because:
/// 1. input types are all defined by blst and `repr(C)` 
/// 2. blst behavior is assumed memory safe
/// 3. The unsafe block is used purely for FFI calls to the BLST library.
#[inline]
pub(super) fn map_fp_to_g1(fp: &blst_fp) -> blst_p1_affine {
    let mut p = blst_p1::default();
    // The third parameter is unused if null.
    unsafe { blst_map_to_g1(&mut p, fp, core::ptr::null()) };
    p1_to_affine(&p)
}
// Test vectors taken from https://eips.ethereum.org/assets/eip-2537/map_fp_to_G1_bls.json and https://eips.ethereum.org/assets/eip-2537/fail-map_fp_to_G1_bls.json
#[cfg(test)]
mod tests {
    use super::*;
    use crate::interpreter::System;
    use fil_actors_runtime::test_utils::MockRuntime;
    use hex_literal::hex;
    use substrate_bn::CurveError;

    #[test]
    fn test_map_fp_to_g1_success() {
        let rt = MockRuntime::default();
        rt.in_call.replace(true);
        let mut system = System::create(&rt).unwrap();

        // Test case 1: bls_g1map_
        let input1 = hex!(
            "00000000000000000000000000000000156c8a6a2c184569d69a76be144b5cdc5141d2d2ca4fe341f011e25e3969c55ad9e9b9ce2eb833c81a908e5fa4ac5f03"
        );
        let expected1 = hex!(
            "00000000000000000000000000000000184bb665c37ff561a89ec2122dd343f20e0f4cbcaec84e3c3052ea81d1834e192c426074b02ed3dca4e7676ce4ce48ba\
             0000000000000000000000000000000004407b8d35af4dacc809927071fc0405218f1401a6d15af775810e4e460064bcc9468beeba82fdc751be70476c888bf3"
        );
        let res = bls12_map_fp_to_g1(&mut system, &input1, PrecompileContext::default()).unwrap();
        assert_eq!(res, expected1, "Test case 1 failed");

        // Test case 2: bls_g1map_616263
        let input2 = hex!(
            "00000000000000000000000000000000147e1ed29f06e4c5079b9d14fc89d2820d32419b990c1c7bb7dbea2a36a045124b31ffbde7c99329c05c559af1c6cc82"
        );
        let expected2 = hex!(
            "00000000000000000000000000000000009769f3ab59bfd551d53a5f846b9984c59b97d6842b20a2c565baa167945e3d026a3755b6345df8ec7e6acb6868ae6d\
             000000000000000000000000000000001532c00cf61aa3d0ce3e5aa20c3b531a2abd2c770a790a2613818303c6b830ffc0ecf6c357af3317b9575c567f11cd2c"
        );
        let res = bls12_map_fp_to_g1(&mut system, &input2, PrecompileContext::default()).unwrap();
        assert_eq!(res, expected2, "Test case 2 failed");

        // Test case 3: bls_g1map_6162636465663031
        let input3 = hex!(
            "0000000000000000000000000000000004090815ad598a06897dd89bcda860f25837d54e897298ce31e6947378134d3761dc59a572154963e8c954919ecfa82d"
        );
        let expected3 = hex!(
            "000000000000000000000000000000001974dbb8e6b5d20b84df7e625e2fbfecb2cdb5f77d5eae5fb2955e5ce7313cae8364bc2fff520a6c25619739c6bdcb6a\
             0000000000000000000000000000000015f9897e11c6441eaa676de141c8d83c37aab8667173cbe1dfd6de74d11861b961dccebcd9d289ac633455dfcc7013a3"
        );
        let res = bls12_map_fp_to_g1(&mut system, &input3, PrecompileContext::default()).unwrap();
        assert_eq!(res, expected3, "Test case 3 failed");

        // Test case 4: bls_g1map_713132385f717171
        let input4 = hex!(
            "0000000000000000000000000000000008dccd088ca55b8bfbc96fb50bb25c592faa867a8bb78d4e94a8cc2c92306190244532e91feba2b7fed977e3c3bb5a1f"
        );
        let expected4 = hex!(
            "000000000000000000000000000000000a7a047c4a8397b3446450642c2ac64d7239b61872c9ae7a59707a8f4f950f101e766afe58223b3bff3a19a7f754027c\
             000000000000000000000000000000001383aebba1e4327ccff7cf9912bda0dbc77de048b71ef8c8a81111d71dc33c5e3aa6edee9cf6f5fe525d50cc50b77cc9"
        );
        let res = bls12_map_fp_to_g1(&mut system, &input4, PrecompileContext::default()).unwrap();
        assert_eq!(res, expected4, "Test case 4 failed");

        // Test case 5: bls_g1map_613531325f616161
        let input5 = hex!(
            "000000000000000000000000000000000dd824886d2123a96447f6c56e3a3fa992fbfefdba17b6673f9f630ff19e4d326529db37e1c1be43f905bf9202e0278d"
        );
        let expected5 = hex!(
            "000000000000000000000000000000000e7a16a975904f131682edbb03d9560d3e48214c9986bd50417a77108d13dc957500edf96462a3d01e62dc6cd468ef11\
             000000000000000000000000000000000ae89e677711d05c30a48d6d75e76ca9fb70fe06c6dd6ff988683d89ccde29ac7d46c53bb97a59b1901abf1db66052db"
        );
        let res = bls12_map_fp_to_g1(&mut system, &input5, PrecompileContext::default()).unwrap();
        assert_eq!(res, expected5, "Test case 5 failed");
    }

    #[test]
    fn test_map_fp_to_g1_failures() {
        let rt = MockRuntime::default();
        rt.in_call.replace(true);
        let mut system = System::create(&rt).unwrap();

        // Test case 1: Empty input
        let empty_input: Vec<u8> = vec![];
        let res = bls12_map_fp_to_g1(&mut system, &empty_input, PrecompileContext::default());
        assert!(
            matches!(res, Err(PrecompileError::IncorrectInputSize)),
            "Test case 'bls_mapg1_empty_input' failed: Empty input should return IncorrectInputSize error"
        );

        // Test case 2: Short input (48 bytes instead of 64)
        let short_input = hex!(
            "00000000000000000000000000000000156c8a6a2c184569d69a76be144b5cdc5141d2d2ca4fe341f011e25e3969c55ad9e9b9ce2eb833c81a908e5fa4ac5f"
        );
        let res = bls12_map_fp_to_g1(&mut system, &short_input, PrecompileContext::default());
        assert!(
            matches!(res, Err(PrecompileError::IncorrectInputSize)),
            "Test case 'bls_mapg1_short_input' failed: Short input should return IncorrectInputSize error"
        );

        // Test case 3: Large input (65 bytes instead of 64)
        let large_input = hex!(
            "0000000000000000000000000000000000156c8a6a2c184569d69a76be144b5cdc5141d2d2ca4fe341f011e25e3969c55ad9e9b9ce2eb833c81a908e5fa4ac5f03"
        );
        let res = bls12_map_fp_to_g1(&mut system, &large_input, PrecompileContext::default());
        assert!(
            matches!(res, Err(PrecompileError::IncorrectInputSize)),
            "Test case 'bls_mapg1_large_input' failed: Large input should return IncorrectInputSize error"
        );

        // Test case 4: Invalid top bytes (non-zero padding)
        let invalid_top = hex!(
            "1000000000000000000000000000000000156c8a6a2c184569d69a76be144b5cdc5141d2d2ca4fe341f011e25e3969c55ad9e9b9ce2eb833c81a908e5fa4ac5f"
        );
        let res = bls12_map_fp_to_g1(&mut system, &invalid_top, PrecompileContext::default());
        assert!(
            matches!(res, Err(PrecompileError::InvalidInput)),
            "Test case 'bls_mapg1_top_bytes' failed: Invalid top bytes should return InvalidInput error"
        );

        // Test case 5: Invalid field element
        let invalid_field = hex!(
            "000000000000000000000000000000002f6d9c5465982c0421b61e74579709b3b5b91e57bdd4f6015742b4ff301abb7ef895b9cce00c33c7d48f8e5fa4ac09ae"
        );
        let res = bls12_map_fp_to_g1(&mut system, &invalid_field, PrecompileContext::default());
        assert!(
            matches!(res, Err(PrecompileError::EcErr(CurveError::NotMember))),
            "Test case 'bls_invalid_fq_element' failed: Invalid field element should return CurveError error, instead got {:?}",
            res
        );
    }
}
