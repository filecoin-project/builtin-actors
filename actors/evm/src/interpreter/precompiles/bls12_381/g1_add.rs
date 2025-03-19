use crate::interpreter::{
    precompiles::{PrecompileContext, PrecompileError, PrecompileResult},
    System,
};
use fil_actors_runtime::runtime::Runtime;

use crate::interpreter::precompiles::bls_util::{
    G1_ADD_INPUT_LENGTH,
    G1_INPUT_LENGTH,
    G1_OUTPUT_LENGTH,
    encode_g1_point,
    extract_g1_input,
};

use blst::{
    blst_p1,
    blst_p1_add_or_double_affine,
    blst_p1_affine,
    blst_p1_from_affine,
    blst_p1_to_affine,
};

/// BLS12_G1ADD precompile
/// Implements G1 point addition according to EIP-2537
pub fn bls12_g1add<RT: Runtime>(
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
}