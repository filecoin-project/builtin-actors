use crate::interpreter::{
    precompiles::{PrecompileContext, PrecompileError, PrecompileResult},
    System,
};
use fil_actors_runtime::runtime::Runtime;
use substrate_bn::CurveError;

use crate::interpreter::precompiles::bls_util::{
    G2_MSM_INPUT_LENGTH,
    G2_INPUT_ITEM_LENGTH,
    G2_OUTPUT_LENGTH,
    SCALAR_LENGTH,
    NBITS,
    encode_g2_point,
    extract_g2_input,
    extract_scalar_input,
};

use blst::{
    blst_p2,
    blst_p2_affine,
    blst_p2_from_affine,
    blst_p2_to_affine,
    p2_affines,
};

/// BLS12_G2MSM precompile
/// Implements G2 multi-scalar multiplication according to EIP-2537
pub fn bls12_g2msm<RT: Runtime>(
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::interpreter::System;
    use fil_actors_runtime::test_utils::MockRuntime;
    use hex_literal::hex;

    
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