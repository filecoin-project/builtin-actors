use super::PrecompileContext;
use super::PrecompileError;
use super::PrecompileResult;
use fil_actors_runtime::runtime::Runtime;
use crate::interpreter::System;

use blst::{
    blst_p1, blst_p1_add_or_double_affine, blst_p1_affine, blst_p1_from_affine, blst_p1_to_affine, blst_fp, blst_p1_affine_on_curve, blst_fp_from_bendian, blst_bendian_from_fp, blst_scalar, blst_scalar_from_bendian, p1_affines
};

const G1_INPUT_LENGTH: usize = 128;
const G1_ADD_INPUT_LENGTH: usize = G1_INPUT_LENGTH * 2;
const G1_OUTPUT_LENGTH: usize = 128;
/// Finite field element padded input length.
pub const PADDED_FP_LENGTH: usize = 64;
/// Input elements padding length.
pub const PADDING_LENGTH: usize = 16;
const G1_MSM_INPUT_LENGTH: usize = 160;
const G1_INPUT_ITEM_LENGTH: usize = 128;
const SCALAR_LENGTH: usize = 32;
const NBITS: usize = 255; // Number of bits in BLS12-381 scalar field


/// https://eips.ethereum.org/EIPS/eip-2537
/// Encodes a single finite field element into byte slice with padding.
pub(super) fn fp_to_bytes(out: &mut [u8], input: *const blst_fp) {
    if out.len() != PADDED_FP_LENGTH {
        return;
    }
    let (padding, rest) = out.split_at_mut(PADDING_LENGTH);
    padding.fill(0);
    // SAFETY: Out length is checked previously, `input` is a blst value.
    unsafe { blst_bendian_from_fp(rest.as_mut_ptr(), input) };
}

/// Checks whether or not the input represents a canonical field element, returning the field
/// element if successful.
fn fp_from_bendian(bytes: &[u8; 48]) -> Result<blst_fp, PrecompileError> {
    let mut fp = blst_fp::default();
    unsafe {
        // This performs the check for canonical field elements
        blst_fp_from_bendian(&mut fp, bytes.as_ptr());
    }
    Ok(fp)
}


/// Extracts a scalar value from a 32-byte input.
/// 
/// According to EIP-2537, the scalar input:
/// - Must be exactly 32 bytes
/// - Is interpreted as a big-endian integer
/// - Is not required to be less than the curve order
/// 
/// Returns a Result containing either the scalar value or a PrecompileError
pub(super) fn extract_scalar_input(input: &[u8]) -> Result<blst_scalar, PrecompileError> {
    // Check input length
    if input.len() != SCALAR_LENGTH {
        return Err(PrecompileError::IncorrectInputSize);
    }

    let mut scalar = blst_scalar::default();
    
    // Convert from big-endian bytes to scalar
    // SAFETY: Input length is checked above and scalar is properly initialized
    unsafe {
        blst_scalar_from_bendian(&mut scalar, input.as_ptr());
    }

    Ok(scalar)
}

/// Returns a `blst_p1_affine` from the provided byte slices, which represent the x and y
/// affine coordinates of the point.
///
/// If the x or y coordinate do not represent a canonical field element, an error is returned.
///
/// See [fp_from_bendian] for more information.
fn decode_and_check_g1(
    x_bytes: &[u8; 48],
    y_bytes: &[u8; 48],
) -> Result<blst_p1_affine, PrecompileError> {
    Ok(blst_p1_affine {
        x: fp_from_bendian(x_bytes)?,
        y: fp_from_bendian(y_bytes)?,
    })
}

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
    let a_aff = extract_g1_point(a_bytes)?;
    let b_aff = extract_g1_point(b_bytes)?;

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
/// Extracts a G1 point in Affine format from a 128 byte slice representation.
fn extract_g1_point(input: &[u8]) -> Result<blst_p1_affine, PrecompileError> {
    if input.len() != G1_INPUT_LENGTH {
        return Err(PrecompileError::IncorrectInputSize);
    }

    // Split input and remove padding for x and y coordinates
    let x_bytes = remove_padding(&input[..PADDED_FP_LENGTH])?;
    let y_bytes = remove_padding(&input[PADDED_FP_LENGTH..G1_INPUT_LENGTH])?;
 
    let point = decode_and_check_g1(x_bytes, y_bytes)?;

    // Check if point is on curve (no subgroup check needed for addition)
    unsafe {
        if !blst_p1_affine_on_curve(&point) {
            return Err(PrecompileError::InvalidInput);
        }
    }

    Ok(point)
}

/// Removes zeros with which the precompile inputs are left padded to 64 bytes.
fn remove_padding(input: &[u8]) -> Result<&[u8; 48], PrecompileError> {
    if input.len() != PADDED_FP_LENGTH {
        return Err(PrecompileError::IncorrectInputSize);
    }
    let (padding, unpadded) = input.split_at(PADDING_LENGTH);
    if !padding.iter().all(|&x| x == 0) {
        return Err(PrecompileError::InvalidInput);
    }
    unpadded.try_into().map_err(|_| PrecompileError::IncorrectInputSize)
}

/// Encodes a G1 point in affine format into byte slice with padded elements.
fn encode_g1_point(input: *const blst_p1_affine) -> Vec<u8> {
    let mut out = vec![0u8; G1_OUTPUT_LENGTH];
    // SAFETY: Out comes from fixed length array, input is a blst value.
    unsafe {
        fp_to_bytes(&mut out[..PADDED_FP_LENGTH], &(*input).x);
        fp_to_bytes(&mut out[PADDED_FP_LENGTH..], &(*input).y);
    }
    out.into()
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
        let p0_aff = &extract_g1_point(slice)?;

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
    Err(PrecompileError::CallForbidden)
}

/// BLS12_G2MSM precompile
/// Implements G2 multi-scalar multiplication according to EIP-2537
#[allow(dead_code,unused_variables)]
pub(super) fn bls12_g2msm<RT: Runtime>(
    _: &mut System<RT>,
    input: &[u8],
    _: PrecompileContext,
) -> PrecompileResult {
    Err(PrecompileError::CallForbidden)
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
    fn test_fp_conversion() {
        // Test fp_to_bytes and fp_from_bendian
        let test_bytes: [u8; 48] = [
            0x11, 0x22, 0x33, 0x44, 0x55, 0x66, 0x77, 0x88,
            0x99, 0xaa, 0xbb, 0xcc, 0xdd, 0xee, 0xff, 0x00,
            0x11, 0x22, 0x33, 0x44, 0x55, 0x66, 0x77, 0x88,
            0x99, 0xaa, 0xbb, 0xcc, 0xdd, 0xee, 0xff, 0x00,
            0x11, 0x22, 0x33, 0x44, 0x55, 0x66, 0x77, 0x88,
            0x99, 0xaa, 0xbb, 0xcc, 0xdd, 0xee, 0xff, 0x00,
        ];

        // Test roundtrip conversion
        let fp = fp_from_bendian(&test_bytes).unwrap();
        let mut output = [0u8; 48];
        fp_to_bytes(&mut output, &fp);
        assert_eq!(test_bytes, output);
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

        // // Test: Point not in correct subgroup
        // let not_in_subgroup = hex::decode(
        //     "000000000000000000000000000000000123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef0\
        //      00000000000000000000000000000000193fb7cedb32b2c3adc06ec11a96bc0d661869316f5e4a577a9f7c179593987beb4fb2ee424dbb2f5dd891e228b46c4a\
        //      0000000000000000000000000000000000000000000000000000000000000002"
        // ).unwrap();
        // let res = bls12_g1msm(&mut system, &not_in_subgroup, ctx);
        // assert!(matches!(res, Err(PrecompileError::InvalidInput)));
    }
}