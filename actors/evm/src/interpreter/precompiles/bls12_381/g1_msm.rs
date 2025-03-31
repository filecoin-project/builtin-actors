use crate::interpreter::{
    precompiles::{PrecompileContext, PrecompileError, PrecompileResult},
    System,
};
use fil_actors_runtime::runtime::Runtime;


use crate::interpreter::precompiles::bls_util::{
    G1_MSM_INPUT_LENGTH,
    G1_INPUT_ITEM_LENGTH,
    G1_OUTPUT_LENGTH,
    SCALAR_LENGTH,
    NBITS,
    encode_g1_point,
    extract_g1_input,
    extract_scalar_input,
};

use blst::{
    blst_p1,
    blst_p1_affine,
    blst_p1_from_affine,
    blst_p1_to_affine,
    p1_affines,
};

/// Implements EIP-2537 G1MSM precompile.
/// G1 multi-scalar-multiplication call expects `160*k` bytes as an input that is interpreted
/// as byte concatenation of `k` slices each of them being a byte concatenation
/// of encoding of G1 point (`128` bytes) and encoding of a scalar value (`32`
/// bytes).
/// Output is an encoding of multi-scalar-multiplication operation result - single G1
/// point (`128` bytes).
/// See also: <https://eips.ethereum.org/EIPS/eip-2537#abi-for-g1-multiexponentiation>
pub fn bls12_g1msm<RT: Runtime>(
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


#[cfg(test)]
mod tests {
    use super::*;
    use crate::interpreter::System;
    use fil_actors_runtime::test_utils::MockRuntime;
    use substrate_bn::CurveError;

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
    fn test_g1_msm_failure() {
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


        // // Test: Invalid field element
        let invalid_field = hex::decode(
            "0000000000000000000000000000000031f2e5916b17be2e71b10b4292f558e727dfd7d48af9cbc5087f0ce00dcca27c8b01e83eaace1aefb539f00adb2271660000000000000000000000000000000008b3f481e3aaa0f1a09e30ed741d8ae4fcf5e095d5d00af600db18cb2c04b3edd03cc744a2888ae40caa232946c5e7e10000000000000000000000000000000000000000000000000000000000000002"
        ).unwrap();
        let res = bls12_g1msm(&mut system, &invalid_field, ctx);
        assert!(matches!(res, Err(PrecompileError::EcErr(CurveError::NotMember))));

        // Test: Point not on curve
        let not_on_curve = hex::decode(
            "0000000000000000000000000000000017f1d3a73197d7942695638c4fa9ac0fc3688c4f9774b905a14e3a3f171bac586c55e83ff97a1aeffb3af00adb22c6bb\
            00000000000000000000000000000000186b28d92356c4dfec4b5201ad099dbdede3781f8998ddf929b4cd7756192185ca7b8f4ef7088f813270ac3d48868a21\
            0000000000000000000000000000000000000000000000000000000000000002"
        ).unwrap();
        let res = bls12_g1msm(&mut system, &not_on_curve, ctx);
        assert!(matches!(res, Err(PrecompileError::EcErr(CurveError::NotMember))));

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
}