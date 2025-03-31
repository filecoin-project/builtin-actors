use crate::interpreter::{
    precompiles::{PrecompileContext, PrecompileError, PrecompileResult},
    System,
};
use fil_actors_runtime::runtime::Runtime;
use substrate_bn::CurveError;

use crate::interpreter::precompiles::bls_util::{
    G2_ADD_INPUT_LENGTH,
    G2_INPUT_ITEM_LENGTH,
    encode_g2_point,
    extract_g2_input,
};

use blst::{
    blst_p2,
    blst_p2_add_or_double_affine,
    blst_p2_affine,
    blst_p2_from_affine,
    blst_p2_to_affine,
};

/// BLS12_G2ADD precompile
/// Implements G2 point addition according to EIP-2537
pub fn bls12_g2add<RT: Runtime>(
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::interpreter::System;
    use fil_actors_runtime::test_utils::MockRuntime;
    use hex_literal::hex;

    #[test]
    fn test_g2_add_success() {
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
    fn test_g2_add_failure() {
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
        assert!(matches!(res, Err(PrecompileError::EcErr(CurveError::NotMember))),
            "Point not on curve should return InvalidInput error");

        // Test case 5: Invalid field element
        let invalid_field = hex!(
            "000000000000000000000000000000001c4bb49d2a0ef12b7123acdd7110bd292b5bc659edc54dc21b81de057194c79b2a5803255959bbef8e7f56c8c12168630000000000000000000000000000000013e02b6052719f607dacd3a088274f65596bd0d09920b61ab5da61bbdc7f5049334cf11213945d57e5ac7d055d042b7e000000000000000000000000000000000ce5d527727d6e118cc9cdc6da2e351aadfd9baa8cbdd3a76d429a695160d12c923ac9cc3baca289e193548608b82801000000000000000000000000000000000606c4a02ea734cc32acd2b02bc28b99cb3e287e85a763af267492ab572e99ab3f370d275cec1da1aaa9075ff05f79be00000000000000000000000000000000103121a2ceaae586d240843a398967325f8eb5a93e8fea99b62b9f88d8556c80dd726a4b30e84a36eeabaf3592937f2700000000000000000000000000000000086b990f3da2aeac0a36143b7d7c824428215140db1bb859338764cb58458f081d92664f9053b50b3fbd2e4723121b68000000000000000000000000000000000f9e7ba9a86a8f7624aa2b42dcc8772e1af4ae115685e60abc2c9b90242167acef3d0be4050bf935eed7c3b6fc7ba77e000000000000000000000000000000000d22c3652d0dc6f0fc9316e14268477c2049ef772e852108d269d9c38dba1d4802e8dae479818184c08f9a569d878451"
        );
        let res = bls12_g2add(&mut system, &invalid_field, PrecompileContext::default());
        assert!(matches!(res, Err(PrecompileError::EcErr(CurveError::NotMember))),
            "Invalid field element should return InvalidInput error");

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

}