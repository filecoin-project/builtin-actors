use super::PrecompileContext;
use super::PrecompileError;
use super::PrecompileResult;
use fil_actors_runtime::runtime::Runtime;
use crate::interpreter::System;
use super::bls_util::{
    // Constants
    G1_INPUT_LENGTH,
    G1_ADD_INPUT_LENGTH,
    G1_OUTPUT_LENGTH,
    G1_MSM_INPUT_LENGTH,
    G1_INPUT_ITEM_LENGTH,
    G2_ADD_INPUT_LENGTH,
    G2_INPUT_ITEM_LENGTH,
    G2_OUTPUT_LENGTH,
    G2_MSM_INPUT_LENGTH,
    SCALAR_LENGTH,
    NBITS,

    // Functions
    encode_g1_point,
    encode_g2_point,
    extract_g1_input,
    extract_g2_input,
    extract_scalar_input,

};

use blst::{
    blst_p1, blst_p1_add_or_double_affine, blst_p1_affine, blst_p1_from_affine, blst_p1_to_affine, p1_affines, blst_p2, blst_p2_affine, blst_p2_add_or_double_affine, blst_p2_from_affine, blst_p2_to_affine, p2_affines
};

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
    let a_aff = extract_g1_input(a_bytes, false)?;
    let b_aff = extract_g1_input(b_bytes, false)?;

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

/// Implements EIP-2537 G1MSM precompile.
/// G1 multi-scalar-multiplication call expects `160*k` bytes as an input that is interpreted
/// as byte concatenation of `k` slices each of them being a byte concatenation
/// of encoding of G1 point (`128` bytes) and encoding of a scalar value (`32`
/// bytes).
/// Output is an encoding of multi-scalar-multiplication operation result - single G1
/// point (`128` bytes).
/// See also: <https://eips.ethereum.org/EIPS/eip-2537#abi-for-g1-multiexponentiation>
pub(super) fn bls12_g1msm<RT: Runtime>(
    _: &mut System<RT>,
    input: &[u8],
    _: PrecompileContext,
) -> PrecompileResult {
    let input_len = input.len();
    if input_len == 0 || input_len % G1_MSM_INPUT_LENGTH != 0 {
        return Err(PrecompileError::IncorrectInputSize);
    }

    let k = input_len / G1_MSM_INPUT_LENGTH;
    let mut g1_points: Vec<blst_p1> = Vec::with_capacity(k);
    let mut scalars: Vec<u8> = Vec::with_capacity(k * SCALAR_LENGTH);

    // Process each (point, scalar) pair
    for i in 0..k {
        let slice = &input[i * G1_MSM_INPUT_LENGTH..i * G1_MSM_INPUT_LENGTH + G1_INPUT_ITEM_LENGTH];

        // Skip points at infinity (all zeros)
        if slice.iter().all(|i| *i == 0) {
            continue;
        }

        // NB: Scalar multiplications, MSMs and pairings MUST perform a subgroup check.
        //
        // So we set the subgroup_check flag to `true`
        let p0_aff = &extract_g1_input(slice, true)?;

        let mut p0 = blst_p1::default();
        // SAFETY: `p0` and `p0_aff` are blst values.
        unsafe { blst_p1_from_affine(&mut p0, p0_aff) };
        g1_points.push(p0);

        scalars.extend_from_slice(
            &extract_scalar_input(
                &input[i * G1_MSM_INPUT_LENGTH + G1_INPUT_ITEM_LENGTH
                    ..i * G1_MSM_INPUT_LENGTH + G1_INPUT_ITEM_LENGTH + SCALAR_LENGTH],
            )?
            .b,
        );
    }

    // Return infinity point if all points are infinity
    if g1_points.is_empty() {
        return Ok(vec![0u8; G1_OUTPUT_LENGTH]);
    }
    let points = p1_affines::from(&g1_points);
    let multiexp = points.mult(&scalars, NBITS);

    let mut multiexp_aff = blst_p1_affine::default();
    // SAFETY: `multiexp_aff` and `multiexp` are blst values.
    unsafe { blst_p1_to_affine(&mut multiexp_aff, &multiexp) };
    Ok(encode_g1_point(&multiexp_aff))
}


/// BLS12_G2ADD precompile
/// Implements G2 point addition according to EIP-2537
#[allow(dead_code,unused_variables)]
pub(super) fn bls12_g2add<RT: Runtime>(
    _: &mut System<RT>,
    input: &[u8],
    _: PrecompileContext,
) -> PrecompileResult {
    if input.len() != G2_ADD_INPUT_LENGTH {
        return Err(PrecompileError::IncorrectInputSize);
    }

    // Extract the two input G2 points
    // No subgroup check needed for addition
    let a_aff = extract_g2_input(&input[..G2_INPUT_ITEM_LENGTH], false)?;
    let b_aff = extract_g2_input(&input[G2_INPUT_ITEM_LENGTH..], false)?;

    let mut b = blst_p2::default();
    // Convert b_aff to projective coordinates
    unsafe { blst_p2_from_affine(&mut b, &b_aff) };

    let mut p = blst_p2::default();
    // Add the points
    unsafe { blst_p2_add_or_double_affine(&mut p, &b, &a_aff) };

    let mut p_aff = blst_p2_affine::default();
    // Convert result back to affine coordinates
    unsafe { blst_p2_to_affine(&mut p_aff, &p) };

    // Encode the result
    Ok(encode_g2_point(&p_aff))
}

/// BLS12_G2MSM precompile
/// Implements G2 multi-scalar multiplication according to EIP-2537
pub(super) fn bls12_g2msm<RT: Runtime>(
    _: &mut System<RT>,
    input: &[u8],
    _: PrecompileContext,
) -> PrecompileResult {
    let input_len = input.len();
    if input_len == 0 || input_len % G2_MSM_INPUT_LENGTH != 0 {
        return Err(PrecompileError::IncorrectInputSize);
    }

    let k = input_len / G2_MSM_INPUT_LENGTH;
    let mut g2_points: Vec<blst_p2> = Vec::with_capacity(k);
    let mut scalars: Vec<u8> = Vec::with_capacity(k * SCALAR_LENGTH);

    // Process each (point, scalar) pair
    for i in 0..k {
        let slice = &input[i * G2_MSM_INPUT_LENGTH..i * G2_MSM_INPUT_LENGTH + G2_INPUT_ITEM_LENGTH];

        // Skip points at infinity (all zeros)
        if slice.iter().all(|i| *i == 0) {
            continue;
        }

        // NB: Scalar multiplications, MSMs and pairings MUST perform a subgroup check.
        //
        // So we set the subgroup_check flag to `true`
        let p0_aff = extract_g2_input(slice, true)?;

        let mut p0 = blst_p2::default();
        // Convert to projective coordinates
        // SAFETY: `p0` and `p0_aff` are blst values
        unsafe { blst_p2_from_affine(&mut p0, &p0_aff) };
        g2_points.push(p0);

        // Extract and add scalar
        scalars.extend_from_slice(
            &extract_scalar_input(
                &input[i * G2_MSM_INPUT_LENGTH + G2_INPUT_ITEM_LENGTH
                    ..i * G2_MSM_INPUT_LENGTH + G2_INPUT_ITEM_LENGTH + SCALAR_LENGTH],
            )?
            .b,
        );
    }

    // Return infinity point if all points are infinity
    if g2_points.is_empty() {
        return Ok(vec![0u8; G2_OUTPUT_LENGTH]);
    }

    // Convert points to affine representation for batch operation
    let points = p2_affines::from(&g2_points);
    // Perform multi-scalar multiplication
    let multiexp = points.mult(&scalars, NBITS);

    let mut multiexp_aff = blst_p2_affine::default();
    // Convert result back to affine coordinates
    // SAFETY: `multiexp_aff` and `multiexp` are blst values
    unsafe { blst_p2_to_affine(&mut multiexp_aff, &multiexp) };

    // Encode the result
    Ok(encode_g2_point(&multiexp_aff))
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
        // Input: Two valid G1 points P1 and P2
        // Test case: bls_g1add_g1+p1
        let input = hex::decode(
            "0000000000000000000000000000000017f1d3a73197d7942695638c4fa9ac0fc3688c4f9774b905a14e3a3f171bac586c55e83ff97a1aeffb3af00adb22c6bb\
            0000000000000000000000000000000008b3f481e3aaa0f1a09e30ed741d8ae4fcf5e095d5d00af600db18cb2c04b3edd03cc744a2888ae40caa232946c5e7e1\
            00000000000000000000000000000000112b98340eee2777cc3c14163dea3ec97977ac3dc5c70da32e6e87578f44912e902ccef9efe28d4a78b8999dfbca9426\
            00000000000000000000000000000000186b28d92356c4dfec4b5201ad099dbdede3781f8998ddf929b4cd7756192185ca7b8f4ef7088f813270ac3d48868a21"
        ).unwrap();

        // Expected result from Ethereum test suite
        let expected = hex::decode(
            "000000000000000000000000000000000a40300ce2dec9888b60690e9a41d3004fda4886854573974fab73b046d3147ba5b7a5bde85279ffede1b45b3918d82d\
            0000000000000000000000000000000006d3d887e9f53b9ec4eb6cedf5607226754b07c01ace7834f57f3e7315faefb739e59018e22c492006190fba4a870025"
        ).unwrap();

        let res = bls12_g1add(&mut system, &input, PrecompileContext::default()).unwrap();
        assert_eq!(res, expected, 
            "G1 addition result did not match expected output.\nGot: {}\nExpected: {}", 
            hex::encode(&res), hex::encode(&expected)
        );

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
    fn test_g1_msm_success() {
        let rt = MockRuntime::default();
        rt.in_call.replace(true);
        let mut system = System::create(&rt).unwrap();

        // Test case: bls_g1mul_(g1+g1=2*g1)
        let input = hex::decode(
            "0000000000000000000000000000000017f1d3a73197d7942695638c4fa9ac0fc3688c4f9774b905a14e3a3f171bac586c55e83ff97a1aeffb3af00adb22c6bb\
             0000000000000000000000000000000008b3f481e3aaa0f1a09e30ed741d8ae4fcf5e095d5d00af600db18cb2c04b3edd03cc744a2888ae40caa232946c5e7e1\
             0000000000000000000000000000000000000000000000000000000000000002"
        ).unwrap();

        let expected = hex::decode(
            "000000000000000000000000000000000572cbea904d67468808c8eb50a9450c9721db309128012543902d0ac358a62ae28f75bb8f1c7c42c39a8c5529bf0f4e\
             00000000000000000000000000000000166a9d8cabc673a322fda673779d8e3822ba3ecb8670e461f73bb9021d5fd76a4c56d9d4cd16bd1bba86881979749d28"
        ).unwrap();

        let res = bls12_g1msm(&mut system, &input, PrecompileContext::default()).unwrap();
        assert_eq!(res, expected, 
            "G1 MSM result did not match expected output.\nGot: {}\nExpected: {}", 
            hex::encode(&res), hex::encode(&expected)
        );
    }

    #[test]
    fn test_g1_msm_failures() {
        let rt = MockRuntime::default();
        rt.in_call.replace(true);
        let mut system = System::create(&rt).unwrap();
        let ctx = PrecompileContext::default();

        // Test: Empty input
        let res = bls12_g1msm(&mut system, &[], ctx);
        assert!(matches!(res, Err(PrecompileError::IncorrectInputSize)));

        // Test: Short input
        let short_input = hex::decode(
            "00000000000000000000000000000017f1d3a73197d7942695638c4fa9ac0fc3688c4f9774b905a14e3a3f171bac586c55e83ff97a1aeffb3af00adb22c6bb\
             0000000000000000000000000000000008b3f481e3aaa0f1a09e30ed741d8ae4fcf5e095d5d00af600db18cb2c04b3edd03cc744a2888ae40caa232946c5e7e1\
             0000000000000000000000000000000000000000000000000000000000000002"
        ).unwrap();
        let res = bls12_g1msm(&mut system, &short_input, ctx);
        assert!(matches!(res, Err(PrecompileError::IncorrectInputSize)));

        // TODO: Fix this test
        // Error caused by the fact that the input is not padded to 64 bytes and the padding is not removed
        // https://ethereum-magicians.org/t/eip-2537-bls12-precompile-discussion-thread/4187
        // https://github.com/ethereum/EIPs/blob/master/EIPS/eip-2537.md

        // // Test: Invalid field element
        // let invalid_field = hex::decode(
        //     "0000000000000000000000000000000031f2e5916b17be2e71b10b4292f558e727dfd7d48af9cbc5087f0ce00dcca27c8b01e83eaace1aefb539f00adb2271660000000000000000000000000000000008b3f481e3aaa0f1a09e30ed741d8ae4fcf5e095d5d00af600db18cb2c04b3edd03cc744a2888ae40caa232946c5e7e10000000000000000000000000000000000000000000000000000000000000002"
        // ).unwrap();
        // let res = bls12_g1msm(&mut system, &invalid_field, ctx);
        // match res {
        //     Ok(_) => panic!("Expected error for invalid field element, got success"),
        //     Err(e) => {
        //         println!("Got error: {:?}", e);
        //         assert!(matches!(e, PrecompileError::InvalidInput), 
        //             "Expected InvalidInput error, got {:?}", e);
        //     }
        // }
        // assert!(matches!(res, Err(PrecompileError::InvalidInput)));

        // Test: Point not on curve
        let not_on_curve = hex::decode(
            "0000000000000000000000000000000017f1d3a73197d7942695638c4fa9ac0fc3688c4f9774b905a14e3a3f171bac586c55e83ff97a1aeffb3af00adb22c6bb\
             00000000000000000000000000000000186b28d92356c4dfec4b5201ad099dbdede3781f8998ddf929b4cd7756192185ca7b8f4ef7088f813270ac3d48868a21\
             0000000000000000000000000000000000000000000000000000000000000002"
        ).unwrap();
        let res = bls12_g1msm(&mut system, &not_on_curve, ctx);
        assert!(matches!(res, Err(PrecompileError::InvalidInput)));

        // Test: Invalid top bytes
        let invalid_top = hex::decode(
            "1000000000000000000000000000000017f1d3a73197d7942695638c4fa9ac0fc3688c4f9774b905a14e3a3f171bac586c55e83ff97a1aeffb3af00adb22c6bb\
             0000000000000000000000000000000008b3f481e3aaa0f1a09e30ed741d8ae4fcf5e095d5d00af600db18cb2c04b3edd03cc744a2888ae40caa232946c5e7e1\
             0000000000000000000000000000000000000000000000000000000000000002"
        ).unwrap();
        let res = bls12_g1msm(&mut system, &invalid_top, ctx);
        assert!(matches!(res, Err(PrecompileError::InvalidInput)));

        // Test: Point not in correct subgroup
        let not_in_subgroup = hex::decode(
            "000000000000000000000000000000000123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef00000000000000000000000000000000193fb7cedb32b2c3adc06ec11a96bc0d661869316f5e4a577a9f7c179593987beb4fb2ee424dbb2f5dd891e228b46c4a0000000000000000000000000000000000000000000000000000000000000002"
        ).unwrap();
        let res = bls12_g1msm(&mut system, &not_in_subgroup, ctx);
        assert!(matches!(res, Err(PrecompileError::InvalidInput)));
    }

    #[test]
    fn test_g2_add() {
        let rt = MockRuntime::default();
        rt.in_call.replace(true);
        let mut system = System::create(&rt).unwrap();

        // Test case 1: bls_g2add_g2+p2
        let input1 = hex!(
            "00000000000000000000000000000000024aa2b2f08f0a91260805272dc51051c6e47ad4fa403b02b4510b647ae3d1770bac0326a805bbefd48056c8c121bdb8\
             0000000000000000000000000000000013e02b6052719f607dacd3a088274f65596bd0d09920b61ab5da61bbdc7f5049334cf11213945d57e5ac7d055d042b7e\
             000000000000000000000000000000000ce5d527727d6e118cc9cdc6da2e351aadfd9baa8cbdd3a76d429a695160d12c923ac9cc3baca289e193548608b82801\
             000000000000000000000000000000000606c4a02ea734cc32acd2b02bc28b99cb3e287e85a763af267492ab572e99ab3f370d275cec1da1aaa9075ff05f79be\
             00000000000000000000000000000000103121a2ceaae586d240843a398967325f8eb5a93e8fea99b62b9f88d8556c80dd726a4b30e84a36eeabaf3592937f27\
             00000000000000000000000000000000086b990f3da2aeac0a36143b7d7c824428215140db1bb859338764cb58458f081d92664f9053b50b3fbd2e4723121b68\
             000000000000000000000000000000000f9e7ba9a86a8f7624aa2b42dcc8772e1af4ae115685e60abc2c9b90242167acef3d0be4050bf935eed7c3b6fc7ba77e\
             000000000000000000000000000000000d22c3652d0dc6f0fc9316e14268477c2049ef772e852108d269d9c38dba1d4802e8dae479818184c08f9a569d878451"
        );

        let expected1 = hex!(
            "000000000000000000000000000000000b54a8a7b08bd6827ed9a797de216b8c9057b3a9ca93e2f88e7f04f19accc42da90d883632b9ca4dc38d013f71ede4db00000000000000000000000000000000077eba4eecf0bd764dce8ed5f45040dd8f3b3427cb35230509482c14651713282946306247866dfe39a8e33016fcbe520000000000000000000000000000000014e60a76a29ef85cbd69f251b9f29147b67cfe3ed2823d3f9776b3a0efd2731941d47436dc6d2b58d9e65f8438bad073000000000000000000000000000000001586c3c910d95754fef7a732df78e279c3d37431c6a2b77e67a00c7c130a8fcd4d19f159cbeb997a178108fffffcbd20"
        );

        let res = bls12_g2add(&mut system, &input1, PrecompileContext::default()).unwrap();
        assert_eq!(res, expected1, 
            "G2 addition test case 1 failed.\nGot: {}\nExpected: {}", 
            hex::encode(&res), hex::encode(&expected1)
        );

        // Test case 2: bls_g2add_p2+g2 (commutative property test)
        let input2 = hex!(
            "00000000000000000000000000000000103121a2ceaae586d240843a398967325f8eb5a93e8fea99b62b9f88d8556c80dd726a4b30e84a36eeabaf3592937f27\
             00000000000000000000000000000000086b990f3da2aeac0a36143b7d7c824428215140db1bb859338764cb58458f081d92664f9053b50b3fbd2e4723121b68\
             000000000000000000000000000000000f9e7ba9a86a8f7624aa2b42dcc8772e1af4ae115685e60abc2c9b90242167acef3d0be4050bf935eed7c3b6fc7ba77e\
             000000000000000000000000000000000d22c3652d0dc6f0fc9316e14268477c2049ef772e852108d269d9c38dba1d4802e8dae479818184c08f9a569d878451\
             00000000000000000000000000000000024aa2b2f08f0a91260805272dc51051c6e47ad4fa403b02b4510b647ae3d1770bac0326a805bbefd48056c8c121bdb8\
             0000000000000000000000000000000013e02b6052719f607dacd3a088274f65596bd0d09920b61ab5da61bbdc7f5049334cf11213945d57e5ac7d055d042b7e\
             000000000000000000000000000000000ce5d527727d6e118cc9cdc6da2e351aadfd9baa8cbdd3a76d429a695160d12c923ac9cc3baca289e193548608b82801\
             000000000000000000000000000000000606c4a02ea734cc32acd2b02bc28b99cb3e287e85a763af267492ab572e99ab3f370d275cec1da1aaa9075ff05f79be"
        );

        // Should give same result as test case 1 (addition is commutative)
        let res = bls12_g2add(&mut system, &input2, PrecompileContext::default()).unwrap();
        assert_eq!(res, expected1,
            "G2 addition test case 2 (commutativity) failed.\nGot: {}\nExpected: {}", 
            hex::encode(&res), hex::encode(&expected1)
        );

        // Test case 3: bls_g2add_g2_wrong_order+g2 (points not in correct order)
        let input3 = hex!(
            "00000000000000000000000000000000197bfd0342bbc8bee2beced2f173e1a87be576379b343e93232d6cef98d84b1d696e5612ff283ce2cfdccb2cfb65fa0c00000000000000000000000000000000184e811f55e6f9d84d77d2f79102fd7ea7422f4759df5bf7f6331d550245e3f1bcf6a30e3b29110d85e0ca16f9f6ae7a000000000000000000000000000000000f10e1eb3c1e53d2ad9cf2d398b2dc22c5842fab0a74b174f691a7e914975da3564d835cd7d2982815b8ac57f507348f000000000000000000000000000000000767d1c453890f1b9110fda82f5815c27281aba3f026ee868e4176a0654feea41a96575e0c4d58a14dbfbcc05b5010b100000000000000000000000000000000024aa2b2f08f0a91260805272dc51051c6e47ad4fa403b02b4510b647ae3d1770bac0326a805bbefd48056c8c121bdb80000000000000000000000000000000013e02b6052719f607dacd3a088274f65596bd0d09920b61ab5da61bbdc7f5049334cf11213945d57e5ac7d055d042b7e000000000000000000000000000000000ce5d527727d6e118cc9cdc6da2e351aadfd9baa8cbdd3a76d429a695160d12c923ac9cc3baca289e193548608b82801000000000000000000000000000000000606c4a02ea734cc32acd2b02bc28b99cb3e287e85a763af267492ab572e99ab3f370d275cec1da1aaa9075ff05f79be"
        );

        let expected3 = hex!(
            "0000000000000000000000000000000011f00077935238fc57086414804303b20fab5880bc29f35ebda22c13dd44e586c8a889fe2ba799082c8458d861ac10cf0000000000000000000000000000000007318be09b19be000fe5df77f6e664a8286887ad8373005d7f7a203fcc458c28004042780146d3e43fa542d921c69512000000000000000000000000000000001287eab085d6f8a29f1f1aedb5ad9e8546963f0b11865e05454d86b9720c281db567682a233631f63a2794432a5596ae0000000000000000000000000000000012ec87cea1bacb75aa97728bcd64b27c7a42dd2319a2e17fe3837a05f85d089c5ebbfb73c1d08b7007e2b59ec9c8e065"
        );

        let res = bls12_g2add(&mut system, &input3, PrecompileContext::default()).unwrap();
        assert_eq!(res, expected3,
            "G2 addition test case 3 (wrong order) failed.\nGot: {}\nExpected: {}", 
            hex::encode(&res), hex::encode(&expected3)
        );
    }
    #[test]
    fn test_g2_add_fail() {
        let rt = MockRuntime::default();
        rt.in_call.replace(true);
        let mut system = System::create(&rt).unwrap();

        // Test case 1: Empty input
        let empty_input: Vec<u8> = vec![];
        let res = bls12_g2add(&mut system, &empty_input, PrecompileContext::default());
        assert!(matches!(res, Err(PrecompileError::IncorrectInputSize)),
            "Empty input should return IncorrectInputSize error");

        // Test case 2: Short input
        let short_input = hex!(
            "000000000000000000000000000000024aa2b2f08f0a91260805272dc51051c6e47ad4fa403b02b4510b647ae3d1770bac0326a805bbefd48056c8c121bdb8\
             0000000000000000000000000000000013e02b6052719f607dacd3a088274f65596bd0d09920b61ab5da61bbdc7f5049334cf11213945d57e5ac7d055d042b7e\
             000000000000000000000000000000000ce5d527727d6e118cc9cdc6da2e351aadfd9baa8cbdd3a76d429a695160d12c923ac9cc3baca289e193548608b82801\
             000000000000000000000000000000000606c4a02ea734cc32acd2b02bc28b99cb3e287e85a763af267492ab572e99ab3f370d275cec1da1aaa9075ff05f79be"
        );
        let res = bls12_g2add(&mut system, &short_input, PrecompileContext::default());
        assert!(matches!(res, Err(PrecompileError::IncorrectInputSize)),
            "Short input should return IncorrectInputSize error");

        // Test case 3: Long input (extra byte at start)
        let long_input = hex!(
            "0000000000000000000000000000000000024aa2b2f08f0a91260805272dc51051c6e47ad4fa403b02b4510b647ae3d1770bac0326a805bbefd48056c8c121bdb8\
             0000000000000000000000000000000013e02b6052719f607dacd3a088274f65596bd0d09920b61ab5da61bbdc7f5049334cf11213945d57e5ac7d055d042b7e\
             000000000000000000000000000000000ce5d527727d6e118cc9cdc6da2e351aadfd9baa8cbdd3a76d429a695160d12c923ac9cc3baca289e193548608b82801\
             000000000000000000000000000000000606c4a02ea734cc32acd2b02bc28b99cb3e287e85a763af267492ab572e99ab3f370d275cec1da1aaa9075ff05f79be\
             00000000000000000000000000000000103121a2ceaae586d240843a398967325f8eb5a93e8fea99b62b9f88d8556c80dd726a4b30e84a36eeabaf3592937f27\
             00000000000000000000000000000000086b990f3da2aeac0a36143b7d7c824428215140db1bb859338764cb58458f081d92664f9053b50b3fbd2e4723121b68\
             000000000000000000000000000000000f9e7ba9a86a8f7624aa2b42dcc8772e1af4ae115685e60abc2c9b90242167acef3d0be4050bf935eed7c3b6fc7ba77e\
             000000000000000000000000000000000d22c3652d0dc6f0fc9316e14268477c2049ef772e852108d269d9c38dba1d4802e8dae479818184c08f9a569d878451"
        );
        let res = bls12_g2add(&mut system, &long_input, PrecompileContext::default());
        assert!(matches!(res, Err(PrecompileError::IncorrectInputSize)),
            "Long input should return IncorrectInputSize error");

        // Test case 4: Point not on curve
        let not_on_curve = hex!(
            "00000000000000000000000000000000024aa2b2f08f0a91260805272dc51051c6e47ad4fa403b02b4510b647ae3d1770bac0326a805bbefd48056c8c121bdb8\
             00000000000000000000000000000000086b990f3da2aeac0a36143b7d7c824428215140db1bb859338764cb58458f081d92664f9053b50b3fbd2e4723121b68\
             000000000000000000000000000000000ce5d527727d6e118cc9cdc6da2e351aadfd9baa8cbdd3a76d429a695160d12c923ac9cc3baca289e193548608b82801\
             000000000000000000000000000000000606c4a02ea734cc32acd2b02bc28b99cb3e287e85a763af267492ab572e99ab3f370d275cec1da1aaa9075ff05f79be\
             00000000000000000000000000000000103121a2ceaae586d240843a398967325f8eb5a93e8fea99b62b9f88d8556c80dd726a4b30e84a36eeabaf3592937f27\
             00000000000000000000000000000000086b990f3da2aeac0a36143b7d7c824428215140db1bb859338764cb58458f081d92664f9053b50b3fbd2e4723121b68\
             000000000000000000000000000000000f9e7ba9a86a8f7624aa2b42dcc8772e1af4ae115685e60abc2c9b90242167acef3d0be4050bf935eed7c3b6fc7ba77e\
             000000000000000000000000000000000d22c3652d0dc6f0fc9316e14268477c2049ef772e852108d269d9c38dba1d4802e8dae479818184c08f9a569d878451"
        );
        let res = bls12_g2add(&mut system, &not_on_curve, PrecompileContext::default());
        assert!(matches!(res, Err(PrecompileError::InvalidInput)),
            "Point not on curve should return InvalidInput error");

        // // Test case 5: Invalid field element
        // let invalid_field = hex!(
        //     "000000000000000000000000000000001c4bb49d2a0ef12b7123acdd7110bd292b5bc659edc54dc21b81de057194c79b2a5803255959bbef8e7f56c8c12168630000000000000000000000000000000013e02b6052719f607dacd3a088274f65596bd0d09920b61ab5da61bbdc7f5049334cf11213945d57e5ac7d055d042b7e000000000000000000000000000000000ce5d527727d6e118cc9cdc6da2e351aadfd9baa8cbdd3a76d429a695160d12c923ac9cc3baca289e193548608b82801000000000000000000000000000000000606c4a02ea734cc32acd2b02bc28b99cb3e287e85a763af267492ab572e99ab3f370d275cec1da1aaa9075ff05f79be00000000000000000000000000000000103121a2ceaae586d240843a398967325f8eb5a93e8fea99b62b9f88d8556c80dd726a4b30e84a36eeabaf3592937f2700000000000000000000000000000000086b990f3da2aeac0a36143b7d7c824428215140db1bb859338764cb58458f081d92664f9053b50b3fbd2e4723121b68000000000000000000000000000000000f9e7ba9a86a8f7624aa2b42dcc8772e1af4ae115685e60abc2c9b90242167acef3d0be4050bf935eed7c3b6fc7ba77e000000000000000000000000000000000d22c3652d0dc6f0fc9316e14268477c2049ef772e852108d269d9c38dba1d4802e8dae479818184c08f9a569d878451"
        // );
        // let res = bls12_g2add(&mut system, &invalid_field, PrecompileContext::default());
        // assert!(matches!(res, Err(PrecompileError::InvalidInput)),
        //     "Invalid field element should return InvalidInput error");

        // Test case 6: Invalid top bytes
        let invalid_top = hex!(
            "10000000000000000000000000000000024aa2b2f08f0a91260805272dc51051c6e47ad4fa403b02b4510b647ae3d1770bac0326a805bbefd48056c8c121bdb8\
             0000000000000000000000000000000013e02b6052719f607dacd3a088274f65596bd0d09920b61ab5da61bbdc7f5049334cf11213945d57e5ac7d055d042b7e\
             000000000000000000000000000000000ce5d527727d6e118cc9cdc6da2e351aadfd9baa8cbdd3a76d429a695160d12c923ac9cc3baca289e193548608b82801\
             000000000000000000000000000000000606c4a02ea734cc32acd2b02bc28b99cb3e287e85a763af267492ab572e99ab3f370d275cec1da1aaa9075ff05f79be\
             00000000000000000000000000000000103121a2ceaae586d240843a398967325f8eb5a93e8fea99b62b9f88d8556c80dd726a4b30e84a36eeabaf3592937f27\
             00000000000000000000000000000000086b990f3da2aeac0a36143b7d7c824428215140db1bb859338764cb58458f081d92664f9053b50b3fbd2e4723121b68\
             000000000000000000000000000000000f9e7ba9a86a8f7624aa2b42dcc8772e1af4ae115685e60abc2c9b90242167acef3d0be4050bf935eed7c3b6fc7ba77e\
             000000000000000000000000000000000d22c3652d0dc6f0fc9316e14268477c2049ef772e852108d269d9c38dba1d4802e8dae479818184c08f9a569d878451"
        );
        let res = bls12_g2add(&mut system, &invalid_top, PrecompileContext::default());
        assert!(matches!(res, Err(PrecompileError::InvalidInput)),
            "Invalid top bytes should return InvalidInput error");
    
    }

    #[test]
    fn test_g2_msm() {
        let rt = MockRuntime::default();
        rt.in_call.replace(true);
        let mut system = System::create(&rt).unwrap();

        // Test case 1: g2 * 2
        let input1 = hex!(
            "00000000000000000000000000000000024aa2b2f08f0a91260805272dc51051c6e47ad4fa403b02b4510b647ae3d1770bac0326a805bbefd48056c8c121bdb8\
             0000000000000000000000000000000013e02b6052719f607dacd3a088274f65596bd0d09920b61ab5da61bbdc7f5049334cf11213945d57e5ac7d055d042b7e\
             000000000000000000000000000000000ce5d527727d6e118cc9cdc6da2e351aadfd9baa8cbdd3a76d429a695160d12c923ac9cc3baca289e193548608b82801\
             000000000000000000000000000000000606c4a02ea734cc32acd2b02bc28b99cb3e287e85a763af267492ab572e99ab3f370d275cec1da1aaa9075ff05f79be\
             0000000000000000000000000000000000000000000000000000000000000002"
        );

        let expected1 = hex!(
            "000000000000000000000000000000001638533957d540a9d2370f17cc7ed5863bc0b995b8825e0ee1ea1e1e4d00dbae81f14b0bf3611b78c952aacab827a053\
             000000000000000000000000000000000a4edef9c1ed7f729f520e47730a124fd70662a904ba1074728114d1031e1572c6c886f6b57ec72a6178288c47c33577\
             000000000000000000000000000000000468fb440d82b0630aeb8dca2b5256789a66da69bf91009cbfe6bd221e47aa8ae88dece9764bf3bd999d95d71e4c9899\
             000000000000000000000000000000000f6d4552fa65dd2638b361543f887136a43253d9c66c411697003f7a13c308f5422e1aa0a59c8967acdefd8b6e36ccf3"
        );

        let res = bls12_g2msm(&mut system, &input1, PrecompileContext::default()).unwrap();
        assert_eq!(res, expected1,
            "G2 MSM test case 1 (g2 * 2) failed.\nGot: {}\nExpected: {}",
            hex::encode(&res), hex::encode(&expected1)
        );

        // Test case 2: p2 * 2
        let input2 = hex!(
            "00000000000000000000000000000000103121a2ceaae586d240843a398967325f8eb5a93e8fea99b62b9f88d8556c80dd726a4b30e84a36eeabaf3592937f27\
             00000000000000000000000000000000086b990f3da2aeac0a36143b7d7c824428215140db1bb859338764cb58458f081d92664f9053b50b3fbd2e4723121b68\
             000000000000000000000000000000000f9e7ba9a86a8f7624aa2b42dcc8772e1af4ae115685e60abc2c9b90242167acef3d0be4050bf935eed7c3b6fc7ba77e\
             000000000000000000000000000000000d22c3652d0dc6f0fc9316e14268477c2049ef772e852108d269d9c38dba1d4802e8dae479818184c08f9a569d878451\
             0000000000000000000000000000000000000000000000000000000000000002"
        );

        let expected2 = hex!(
            "000000000000000000000000000000000b76fcbb604082a4f2d19858a7befd6053fa181c5119a612dfec83832537f644e02454f2b70d40985ebb08042d1620d4\
             0000000000000000000000000000000019a4a02c0ae51365d964c73be7babb719db1c69e0ddbf9a8a335b5bed3b0a4b070d2d5df01d2da4a3f1e56aae2ec106d\
             000000000000000000000000000000000d18322f821ac72d3ca92f92b000483cf5b7d9e5d06873a44071c4e7e81efd904f210208fe0b9b4824f01c65bc7e6208\
             0000000000000000000000000000000004e563d53609a2d1e216aaaee5fbc14ef460160db8d1fdc5e1bd4e8b54cd2f39abf6f925969fa405efb9e700b01c7085"
        );

        let res = bls12_g2msm(&mut system, &input2, PrecompileContext::default()).unwrap();
        assert_eq!(res, expected2,
            "G2 MSM test case 2 (p2 * 2) failed.\nGot: {}\nExpected: {}",
            hex::encode(&res), hex::encode(&expected2)
        );

        // Test case 3: g2 * 1 (identity operation)
        let input3 = hex!(
            "00000000000000000000000000000000024aa2b2f08f0a91260805272dc51051c6e47ad4fa403b02b4510b647ae3d1770bac0326a805bbefd48056c8c121bdb8\
             0000000000000000000000000000000013e02b6052719f607dacd3a088274f65596bd0d09920b61ab5da61bbdc7f5049334cf11213945d57e5ac7d055d042b7e\
             000000000000000000000000000000000ce5d527727d6e118cc9cdc6da2e351aadfd9baa8cbdd3a76d429a695160d12c923ac9cc3baca289e193548608b82801\
             000000000000000000000000000000000606c4a02ea734cc32acd2b02bc28b99cb3e287e85a763af267492ab572e99ab3f370d275cec1da1aaa9075ff05f79be\
             0000000000000000000000000000000000000000000000000000000000000001"
        );

        let expected3 = hex!(
            "00000000000000000000000000000000024aa2b2f08f0a91260805272dc51051c6e47ad4fa403b02b4510b647ae3d1770bac0326a805bbefd48056c8c121bdb8\
             0000000000000000000000000000000013e02b6052719f607dacd3a088274f65596bd0d09920b61ab5da61bbdc7f5049334cf11213945d57e5ac7d055d042b7e\
             000000000000000000000000000000000ce5d527727d6e118cc9cdc6da2e351aadfd9baa8cbdd3a76d429a695160d12c923ac9cc3baca289e193548608b82801\
             000000000000000000000000000000000606c4a02ea734cc32acd2b02bc28b99cb3e287e85a763af267492ab572e99ab3f370d275cec1da1aaa9075ff05f79be"
        );

        let res = bls12_g2msm(&mut system, &input3, PrecompileContext::default()).unwrap();
        assert_eq!(res, expected3,
            "G2 MSM test case 3 (g2 * 1) failed.\nGot: {}\nExpected: {}",
            hex::encode(&res), hex::encode(&expected3)
        );
        // Test case 4: p2 * 1 (identity operation)
        let input4 = hex!(
            "00000000000000000000000000000000103121a2ceaae586d240843a398967325f8eb5a93e8fea99b62b9f88d8556c80dd726a4b30e84a36eeabaf3592937f27\
            00000000000000000000000000000000086b990f3da2aeac0a36143b7d7c824428215140db1bb859338764cb58458f081d92664f9053b50b3fbd2e4723121b68\
            000000000000000000000000000000000f9e7ba9a86a8f7624aa2b42dcc8772e1af4ae115685e60abc2c9b90242167acef3d0be4050bf935eed7c3b6fc7ba77e\
            000000000000000000000000000000000d22c3652d0dc6f0fc9316e14268477c2049ef772e852108d269d9c38dba1d4802e8dae479818184c08f9a569d878451\
            0000000000000000000000000000000000000000000000000000000000000001"
        );

        let expected4 = hex!(
            "00000000000000000000000000000000103121a2ceaae586d240843a398967325f8eb5a93e8fea99b62b9f88d8556c80dd726a4b30e84a36eeabaf3592937f27\
            00000000000000000000000000000000086b990f3da2aeac0a36143b7d7c824428215140db1bb859338764cb58458f081d92664f9053b50b3fbd2e4723121b68\
            000000000000000000000000000000000f9e7ba9a86a8f7624aa2b42dcc8772e1af4ae115685e60abc2c9b90242167acef3d0be4050bf935eed7c3b6fc7ba77e\
            000000000000000000000000000000000d22c3652d0dc6f0fc9316e14268477c2049ef772e852108d269d9c38dba1d4802e8dae479818184c08f9a569d878451"
        );

        let res = bls12_g2msm(&mut system, &input4, PrecompileContext::default()).unwrap();
        assert_eq!(res, expected4,
            "G2 MSM test case 4 (p2 * 1) failed.\nGot: {}\nExpected: {}",
            hex::encode(&res), hex::encode(&expected4)
        );

        // Test case 5: g2 * 0 (multiplication by zero)
        let input5 = hex!(
            "00000000000000000000000000000000024aa2b2f08f0a91260805272dc51051c6e47ad4fa403b02b4510b647ae3d1770bac0326a805bbefd48056c8c121bdb8\
            0000000000000000000000000000000013e02b6052719f607dacd3a088274f65596bd0d09920b61ab5da61bbdc7f5049334cf11213945d57e5ac7d055d042b7e\
            000000000000000000000000000000000ce5d527727d6e118cc9cdc6da2e351aadfd9baa8cbdd3a76d429a695160d12c923ac9cc3baca289e193548608b82801\
            000000000000000000000000000000000606c4a02ea734cc32acd2b02bc28b99cb3e287e85a763af267492ab572e99ab3f370d275cec1da1aaa9075ff05f79be\
            0000000000000000000000000000000000000000000000000000000000000000"
        );

        let expected5 = hex!(
            "00000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000\
            00000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000\
            00000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000\
            00000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000"
        );

        let res = bls12_g2msm(&mut system, &input5, PrecompileContext::default()).unwrap();
        assert_eq!(res, expected5,
            "G2 MSM test case 5 (g2 * 0) failed.\nGot: {}\nExpected: {}",
            hex::encode(&res), hex::encode(&expected5)
        );

        // // Test case 6: p2 * 0 (multiplication by zero)
        // let input6 = hex!(
        //     "00000000000000000000000000000000103121a2ceaae586d240843a398967325f8eb5a93e8fea99b62b9f88d8556c80dd726a4b30e84a36eeabaf3592937f27\
        //     00000000000000000000000000000000086b990f3da2aeac0a36143b7d7c824428215140db1bb859338764cb58458f081d92664f9053b50b3fbd2e4723121b68\
        //     000000000000000000000000000000000f9e7ba9a86a8f7624aa2b42dcc8772e1af4ae115685e60abc2c9b90242167acef3d0be4050bf935eed7c3b6fc7ba77e\
        //     000000000000000000000000000000000d22c3652d0dc6f0fc9316e14268477c2049ef772e852108d269d9c38dba1d4802e8dae479818184c08f9a569d878451\
        //     0000000000000000000000000000000000000000000000000000000000000000"
        // );

        // let expected6 = hex!(
        //     "00000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000\
        //     00000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000\
        //     00000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000\
        //     00000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000"
        // );

        // let res = bls12_g2msm(&mut system, &input6, PrecompileContext::default()).unwrap();
        // assert_eq!(res, expected6,
        //     "G2 MSM test case 6 (p2 * 0) failed.\nGot: {}\nExpected: {}",
        //     hex::encode(&res), hex::encode(&expected6)
        // );

        // // Test case 7: infinity * x
        // let input7 = hex!(
        //     "00000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000\
        //     00000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000\
        //     00000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000\
        //     00000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000\
        //     0000000000000000000000000000000000000000000000000000000000000011"
        // );

        // let expected7 = hex!(
        //     "00000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000\
        //     00000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000\
        //     00000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000\
        //     00000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000"
        // );

        // let res = bls12_g2msm(&mut system, &input7, PrecompileContext::default()).unwrap();
        // assert_eq!(res, expected7,
        //     "G2 MSM test case 7 (infinity * x) failed.\nGot: {}\nExpected: {}",
        //     hex::encode(&res), hex::encode(&expected7)
        // );
        // // Test case 8: Random scalar * g2
        // let input8 = hex!(
        //     "00000000000000000000000000000000024aa2b2f08f0a91260805272dc51051c6e47ad4fa403b02b4510b647ae3d1770bac0326a805bbefd48056c8c121bdb8\
        //     0000000000000000000000000000000013e02b6052719f607dacd3a088274f65596bd0d09920b61ab5da61bbdc7f5049334cf11213945d57e5ac7d055d042b7e\
        //     000000000000000000000000000000000ce5d527727d6e118cc9cdc6da2e351aadfd9baa8cbdd3a76d429a695160d12c923ac9cc3baca289e193548608b82801\
        //     000000000000000000000000000000000606c4a02ea734cc32acd2b02bc28b99cb3e287e85a763af267492ab572e99ab3f370d275cec1da1aaa9075ff05f79be\
        //     263dbd792f5b1be47ed85f8938c0f29586af0d3ac7b977f21c278fe1462040e3"
        // );

        // let expected8 = hex!(
        //     "0000000000000000000000000000000014856c22d8cdb2967c720e963eedc999e738373b14172f06fc915769d3cc5ab7ae0a1b9c38f48b5585fb09d4bd2733bb\
        //     000000000000000000000000000000000c400b70f6f8cd35648f5c126cce5417f3be4d8eefbd42ceb4286a14df7e03135313fe5845e3a575faab3e8b949d2488\
        //     00000000000000000000000000000000149a0aacc34beba2beb2f2a19a440166e76e373194714f108e4ab1c3fd331e80f4e73e6b9ea65fe3ec96d7136de81544\
        //     000000000000000000000000000000000e4622fef26bdb9b1e8ef6591a7cc99f5b73164500c1ee224b6a761e676b8799b09a3fd4fa7e242645cc1a34708285e4"
        // );

        // let res = bls12_g2msm(&mut system, &input8, PrecompileContext::default()).unwrap();
        // assert_eq!(res, expected8,
        //     "G2 MSM test case 8 (random scalar * g2) failed.\nGot: {}\nExpected: {}",
        //     hex::encode(&res), hex::encode(&expected8)
        // );

        // // Test case 9: Random scalar * p2
        // let input9 = hex!(
        //     "00000000000000000000000000000000103121a2ceaae586d240843a398967325f8eb5a93e8fea99b62b9f88d8556c80dd726a4b30e84a36eeabaf3592937f27\
        //     00000000000000000000000000000000086b990f3da2aeac0a36143b7d7c824428215140db1bb859338764cb58458f081d92664f9053b50b3fbd2e4723121b68\
        //     000000000000000000000000000000000f9e7ba9a86a8f7624aa2b42dcc8772e1af4ae115685e60abc2c9b90242167acef3d0be4050bf935eed7c3b6fc7ba77e\
        //     000000000000000000000000000000000d22c3652d0dc6f0fc9316e14268477c2049ef772e852108d269d9c38dba1d4802e8dae479818184c08f9a569d878451\
        //     263dbd792f5b1be47ed85f8938c0f29586af0d3ac7b977f21c278fe1462040e3"
        // );

        // let expected9 = hex!(
        //     "00000000000000000000000000000000036074dcbbd0e987531bfe0e45ddfbe09fd015665990ee0c352e8e403fe6af971d8f42141970d9ab14b4dd04874409e6\
        //     00000000000000000000000000000000019705637f24ba2f398f32c3a3e20d6a1cd0fd63e6f8f071cf603a8334f255744927e7bfdfdb18519e019c49ff6e9145\
        //     00000000000000000000000000000000008e74fcff4c4278c9accfb60809ed69bbcbe3d6213ef2304e078d15ec7d6decb4f462b24b8e7cc38cc11b6f2c9e0486\
        //     0000000000000000000000000000000001331d40100f38c1070afd832445881b47cf4d63894666d9907c85ac66604aab5ad329980938cc3c167ccc5b6bc1b8f30"
        // );

        // let res = bls12_g2msm(&mut system, &input9, PrecompileContext::default()).unwrap();
        // assert_eq!(res, expected9,
        //     "G2 MSM test case 9 (random scalar * p2) failed.\nGot: {}\nExpected: {}",
        //     hex::encode(&res), hex::encode(&expected9)
        // );
    }
}