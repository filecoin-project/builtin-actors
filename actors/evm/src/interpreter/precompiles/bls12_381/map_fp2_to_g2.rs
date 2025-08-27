use crate::interpreter::{
    System,
    precompiles::{PrecompileContext, PrecompileError, PrecompileResult},
};
use fil_actors_runtime::runtime::Runtime;

use crate::interpreter::precompiles::bls_util::{
    PADDED_FP_LENGTH, PADDED_FP2_LENGTH, encode_g2_point, p2_to_affine, read_fp2, remove_padding,
};

use blst::{blst_fp2, blst_map_to_g2, blst_p2, blst_p2_affine};

/// **BLS12_MAP_FP2_TO_G2 Precompile**
///
/// Implements mapping of a field element in Fp2 to a G2 point according to
/// [EIP-2537](https://eips.ethereum.org/EIPS/eip-2537#abi-for-g2-multiexponentiation).
///
/// The input must be exactly **128 bytes** (i.e. a padded Fp2 element consisting of
/// two 64‑byte components). The output is the 128‑byte encoding of the resulting G2 point.
pub fn bls12_map_fp2_to_g2<RT: Runtime>(
    _: &mut System<RT>,
    input: &[u8],
    _: PrecompileContext,
) -> PrecompileResult {
    // Ensure the input is exactly PADDED_FP2_LENGTH (128 bytes).
    if input.len() != PADDED_FP2_LENGTH {
        return Err(PrecompileError::IncorrectInputSize);
    }

    // Split input into two 64-byte components.
    let input_c0 = &input[..PADDED_FP_LENGTH];
    let input_c1 = &input[PADDED_FP_LENGTH..PADDED_FP2_LENGTH];

    // Remove padding from both components.
    let c0_bytes = remove_padding(input_c0)?;
    let c1_bytes = remove_padding(input_c1)?;

    // Read the Fp2 element from its two components.
    let fp2 = read_fp2(c0_bytes, c1_bytes)?;

    // Map the Fp2 element to a G2 point.
    let p_aff = map_fp2_to_g2(&fp2);

    // Encode the resulting G2 point and return.
    Ok(encode_g2_point(&p_aff))
}

/// Maps an Fp2 field element to a G2 point (affine form).
///
/// Note: While this function contains an unsafe block for BLST operations,
/// the function itself is safe because:
/// 1. input types are all defined by blst and `repr(C)`
/// 2. blst behavior is assumed memory safe
/// 3. The unsafe block is used purely for FFI calls to the BLST library.
#[inline]
pub(super) fn map_fp2_to_g2(fp2: &blst_fp2) -> blst_p2_affine {
    let mut p = blst_p2::default();
    // SAFETY: `p` and `fp2` are blst values
    // The third argument is unused if null.
    unsafe { blst_map_to_g2(&mut p, fp2, core::ptr::null()) };
    p2_to_affine(&p)
}
// Test vectors taken from https://eips.ethereum.org/assets/eip-2537/map_fp2_to_G2_bls.json and https://eips.ethereum.org/assets/eip-2537/fail-map_fp2_to_G2_bls.json
#[cfg(test)]
mod tests {
    use super::*;
    use crate::interpreter::System;
    use fil_actors_runtime::test_utils::MockRuntime;
    use hex_literal::hex;
    use substrate_bn::CurveError;

    #[test]
    fn test_map_fp2_to_g2_success() {
        let rt = MockRuntime::default();
        rt.in_call.replace(true);
        let mut system = System::create(&rt).unwrap();

        // Test case 1: bls_g2map_
        let input1 = hex!(
            "0000000000000000000000000000000007355d25caf6e7f2f0cb2812ca0e513bd026ed09dda65b177500fa31714e09ea0ded3a078b526bed3307f804d4b93b04\
             0000000000000000000000000000000002829ce3c021339ccb5caf3e187f6370e1e2a311dec9b75363117063ab2015603ff52c3d3b98f19c2f65575e99e8b78c"
        );
        let expected1 = hex!(
            "0000000000000000000000000000000000e7f4568a82b4b7dc1f14c6aaa055edf51502319c723c4dc2688c7fe5944c213f510328082396515734b6612c4e7bb7\
             00000000000000000000000000000000126b855e9e69b1f691f816e48ac6977664d24d99f8724868a184186469ddfd4617367e94527d4b74fc86413483afb35b\
             000000000000000000000000000000000caead0fd7b6176c01436833c79d305c78be307da5f6af6c133c47311def6ff1e0babf57a0fb5539fce7ee12407b0a42\
             000000000000000000000000000000001498aadcf7ae2b345243e281ae076df6de84455d766ab6fcdaad71fab60abb2e8b980a440043cd305db09d283c895e3d"
        );
        let res = bls12_map_fp2_to_g2(&mut system, &input1, PrecompileContext::default()).unwrap();
        assert_eq!(res, expected1, "Test case 'bls_g2map_' failed");

        // Test case 2: bls_g2map_616263
        let input2 = hex!(
            "00000000000000000000000000000000138879a9559e24cecee8697b8b4ad32cced053138ab913b99872772dc753a2967ed50aabc907937aefb2439ba06cc50c\
             000000000000000000000000000000000a1ae7999ea9bab1dcc9ef8887a6cb6e8f1e22566015428d220b7eec90ffa70ad1f624018a9ad11e78d588bd3617f9f2"
        );
        let expected2 = hex!(
            "00000000000000000000000000000000108ed59fd9fae381abfd1d6bce2fd2fa220990f0f837fa30e0f27914ed6e1454db0d1ee957b219f61da6ff8be0d6441f\
             000000000000000000000000000000000296238ea82c6d4adb3c838ee3cb2346049c90b96d602d7bb1b469b905c9228be25c627bffee872def773d5b2a2eb57d\
             00000000000000000000000000000000033f90f6057aadacae7963b0a0b379dd46750c1c94a6357c99b65f63b79e321ff50fe3053330911c56b6ceea08fee656\
             00000000000000000000000000000000153606c417e59fb331b7ae6bce4fbf7c5190c33ce9402b5ebe2b70e44fca614f3f1382a3625ed5493843d0b0a652fc3f"
        );
        let res = bls12_map_fp2_to_g2(&mut system, &input2, PrecompileContext::default()).unwrap();
        assert_eq!(res, expected2, "Test case 'bls_g2map_616263' failed");

        // Test case 3: bls_g2map_6162636465663031
        let input3 = hex!(
            "0000000000000000000000000000000018c16fe362b7dbdfa102e42bdfd3e2f4e6191d479437a59db4eb716986bf08ee1f42634db66bde97d6c16bbfd342b3b8\
             000000000000000000000000000000000e37812ce1b146d998d5f92bdd5ada2a31bfd63dfe18311aa91637b5f279dd045763166aa1615e46a50d8d8f475f184e"
        );
        let expected3 = hex!(
            "00000000000000000000000000000000038af300ef34c7759a6caaa4e69363cafeed218a1f207e93b2c70d91a1263d375d6730bd6b6509dcac3ba5b567e85bf3\
             000000000000000000000000000000000da75be60fb6aa0e9e3143e40c42796edf15685cafe0279afd2a67c3dff1c82341f17effd402e4f1af240ea90f4b659b\
             0000000000000000000000000000000019b148cbdf163cf0894f29660d2e7bfb2b68e37d54cc83fd4e6e62c020eaa48709302ef8e746736c0e19342cc1ce3df4\
             000000000000000000000000000000000492f4fed741b073e5a82580f7c663f9b79e036b70ab3e51162359cec4e77c78086fe879b65ca7a47d34374c8315ac5e"
        );
        let res = bls12_map_fp2_to_g2(&mut system, &input3, PrecompileContext::default()).unwrap();
        assert_eq!(res, expected3, "Test case 'bls_g2map_6162636465663031' failed");

        // Test case 4: bls_g2map_713132385f717171
        let input4 = hex!(
            "0000000000000000000000000000000008d4a0997b9d52fecf99427abb721f0fa779479963315fe21c6445250de7183e3f63bfdf86570da8929489e421d4ee95\
             0000000000000000000000000000000016cb4ccad91ec95aab070f22043916cd6a59c4ca94097f7f510043d48515526dc8eaaea27e586f09151ae613688d5a89"
        );
        let expected4 = hex!(
            "000000000000000000000000000000000c5ae723be00e6c3f0efe184fdc0702b64588fe77dda152ab13099a3bacd3876767fa7bbad6d6fd90b3642e902b208f9\
             0000000000000000000000000000000012c8c05c1d5fc7bfa847f4d7d81e294e66b9a78bc9953990c358945e1f042eedafce608b67fdd3ab0cb2e6e263b9b1ad\
             0000000000000000000000000000000004e77ddb3ede41b5ec4396b7421dd916efc68a358a0d7425bddd253547f2fb4830522358491827265dfc5bcc1928a569\
             0000000000000000000000000000000011c624c56dbe154d759d021eec60fab3d8b852395a89de497e48504366feedd4662d023af447d66926a28076813dd646"
        );
        let res = bls12_map_fp2_to_g2(&mut system, &input4, PrecompileContext::default()).unwrap();
        assert_eq!(res, expected4, "Test case 'bls_g2map_713132385f717171' failed");

        // Test case 5: bls_g2map_613531325f616161
        let input5 = hex!(
            "0000000000000000000000000000000003f80ce4ff0ca2f576d797a3660e3f65b274285c054feccc3215c879e2c0589d376e83ede13f93c32f05da0f68fd6a1000000000000000000000000000000000006488a837c5413746d868d1efb7232724da10eca410b07d8b505b9363bdccf0a1fc0029bad07d65b15ccfe6dd25e20d"
        );
        let expected5 = hex!(
            "000000000000000000000000000000000ea4e7c33d43e17cc516a72f76437c4bf81d8f4eac69ac355d3bf9b71b8138d55dc10fd458be115afa798b55dac34be1000000000000000000000000000000001565c2f625032d232f13121d3cfb476f45275c303a037faa255f9da62000c2c864ea881e2bcddd111edc4a3c0da3e88d00000000000000000000000000000000043b6f5fe4e52c839148dc66f2b3751e69a0f6ebb3d056d6465d50d4108543ecd956e10fa1640dfd9bc0030cc2558d28000000000000000000000000000000000f8991d2a1ad662e7b6f58ab787947f1fa607fce12dde171bc17903b012091b657e15333e11701edcf5b63ba2a561247"
        );
        let res = bls12_map_fp2_to_g2(&mut system, &input5, PrecompileContext::default()).unwrap();
        assert_eq!(res, expected5, "Test case 'bls_g2map_613531325f616161' failed");
    }

    #[test]
    fn test_map_fp2_to_g2_failure() {
        let rt = MockRuntime::default();
        rt.in_call.replace(true);
        let mut system = System::create(&rt).unwrap();

        // Test case 1: Empty input
        let input1: Vec<u8> = vec![];
        let res = bls12_map_fp2_to_g2(&mut system, &input1, PrecompileContext::default());
        assert!(
            matches!(res, Err(PrecompileError::IncorrectInputSize)),
            "Test case 'bls_mapg2_empty_input' failed: expected 'invalid input length'"
        );

        // Test case 2: Short input
        let input2 = hex!(
            "0000000000000000000000000000000007355d25caf6e7f2f0cb2812ca0e513bd026ed09dda65b177500fa31714e09ea0ded3a078b526bed3307f804d4b93b04\
             0000000000000000000000000000000002829ce3c021339ccb5caf3e187f6370e1e2a311dec9b75363117063ab2015603ff52c3d3b98f19c2f65575e99e8b7"
        );
        let res = bls12_map_fp2_to_g2(&mut system, &input2, PrecompileContext::default());
        assert!(
            matches!(res, Err(PrecompileError::IncorrectInputSize)),
            "Test case 'bls_mapg2_short_input' failed: expected 'invalid input length'"
        );

        // Test case 3: Long input
        let input3 = hex!(
            "000000000000000000000000000000000007355d25caf6e7f2f0cb2812ca0e513bd026ed09dda65b177500fa31714e09ea0ded3a078b526bed3307f804d4b93b04\
             0000000000000000000000000000000002829ce3c021339ccb5caf3e187f6370e1e2a311dec9b75363117063ab2015603ff52c3d3b98f19c2f65575e99e8b78c"
        );
        let res = bls12_map_fp2_to_g2(&mut system, &input3, PrecompileContext::default());
        assert!(
            matches!(res, Err(PrecompileError::IncorrectInputSize)),
            "Test case 'bls_mapg2_long_input' failed: expected 'invalid input length'"
        );

        // Test case 4: Invalid top bytes
        let input4 = hex!(
            "000000000000000000000000000000000007355d25caf6e7f2f0cb2812ca0e513bd026ed09dda65b177500fa31714e09ea0ded3a078b526bed3307f804d4b93b04\
             0000000000000000000000000000000002829ce3c021339ccb5caf3e187f6370e1e2a311dec9b75363117063ab2015603ff52c3d3b98f19c2f65575e99e8b7"
        );
        let res = bls12_map_fp2_to_g2(&mut system, &input4, PrecompileContext::default());
        assert!(
            matches!(res, Err(PrecompileError::InvalidInput)),
            "Test case 'bls_mapg2_top_bytes' failed: expected 'invalid field element top bytes'"
        );

        // Test case 5: Invalid field element
        let input5 = hex!(
            "0000000000000000000000000000000021366f100476ce8d3be6cfc90d59fe13349e388ed12b6dd6dc31ccd267ff000e2c993a063ca66beced06f804d4b8e5af\
             0000000000000000000000000000000002829ce3c021339ccb5caf3e187f6370e1e2a311dec9b75363117063ab2015603ff52c3d3b98f19c2f65575e99e8b78c"
        );
        let res = bls12_map_fp2_to_g2(&mut system, &input5, PrecompileContext::default());
        assert!(
            matches!(res, Err(PrecompileError::EcErr(CurveError::NotMember))),
            "Test case 'bls_mapg2_invalid_fq_element' failed: expected 'invalid fp.Element encoding'"
        );
    }
}
