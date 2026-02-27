//! # secp256r1 (P-256) Precompile (`P256VERIFY`, `0x0100`)
//!
//! This module implements secp256r1 (P-256) ECDSA signature verification for the FEVM precompile at `0x0100`.
//!
//! The precompile is specified by Ethereum's [EIP-7951](https://eips.ethereum.org/EIPS/eip-7951) and is interface-compatible
//! with [RIP-7212](https://github.com/ethereum/RIPs/blob/master/RIPS/rip-7212.md).
//!
//! The main purpose of this precompile is to verify ECDSA signatures that use the secp256r1, or
//! P256 elliptic curve. The [`p256_verify`] function represents the implementation of this
//! precompile.
use fil_actors_runtime::runtime::Runtime;
use p256::{
    EncodedPoint,
    ecdsa::{Signature, VerifyingKey, signature::hazmat::PrehashVerifier},
};

use alloy_core::primitives::{B256, B512};

use super::{PrecompileContext, PrecompileResult};
use crate::interpreter::System;

/// P256 verify precompile function
pub fn p256_verify<RT: Runtime>(
    _system: &mut System<RT>,
    input: &[u8],
    _context: PrecompileContext,
) -> PrecompileResult {
    p256_verify_inner(input)
}

/// The input is encoded as follows:
///
/// | signed message hash |  r  |  s  | public key x | public key y |
/// | :-----------------: | :-: | :-: | :----------: | :----------: |
/// |          32         | 32  | 32  |     32       |      32      |
fn p256_verify_inner(input: &[u8]) -> PrecompileResult {
    if verify_impl(input) {
        // Return 32 bytes with last byte set to 1 for success
        Ok(B256::with_last_byte(1).to_vec())
    } else {
        // Return empty vector for failure
        Ok(vec![])
    }
}

/// Returns `true` if the signature included in the input byte slice is
/// valid, `false` otherwise.
pub fn verify_impl(input: &[u8]) -> bool {
    if input.len() != 160 {
        return false;
    }

    // msg signed (msg is already the hash of the original message)
    let msg = <&B256>::try_from(&input[..32]).unwrap();
    // r, s: signature
    let sig = <&B512>::try_from(&input[32..96]).unwrap();
    // x, y: public key
    let pk = <&B512>::try_from(&input[96..160]).unwrap();

    verify_signature(msg.0, sig.0, pk.0).is_some()
}

pub(crate) fn verify_signature(msg: [u8; 32], sig: [u8; 64], pk: [u8; 64]) -> Option<()> {
    // Can fail only if the input is not exact length.
    let signature = Signature::from_slice(&sig).ok()?;
    // Decode the public key bytes (x,y coordinates) using EncodedPoint
    let encoded_point = EncodedPoint::from_untagged_bytes(&pk.into());
    // Create VerifyingKey from the encoded point
    let public_key = VerifyingKey::from_encoded_point(&encoded_point).ok()?;

    public_key.verify_prehash(&msg, &signature).ok()
}

#[cfg(test)]
mod test {
    use super::*;
    use crate::interpreter::System;
    use crate::interpreter::precompiles::PrecompileContext;
    use fil_actors_runtime::test_utils::MockRuntime;
    use serde::Deserialize;

    const TESTDATA_PATH: &str =
        concat!(env!("CARGO_MANIFEST_DIR"), "/precompile-testdata/eip7951_p256verify.json");

    #[derive(Debug, Deserialize)]
    #[serde(rename_all = "PascalCase")]
    struct Eip7951TestVector {
        name: String,
        input: String,
        expected: String,
        gas: u64,
        #[serde(default)]
        no_benchmark: bool,
    }

    fn load_test_vectors() -> Vec<Eip7951TestVector> {
        let testdata = std::fs::read_to_string(TESTDATA_PATH)
            .expect("failed to read EIP-7951 test vector file");
        serde_json::from_str(&testdata).expect("failed to parse EIP-7951 test vectors")
    }

    #[test]
    fn eip7951_vectors_conformance() {
        let rt = MockRuntime::default();
        rt.in_call.replace(true);
        let mut system = System::create(&rt).unwrap();

        let test_vectors = load_test_vectors();
        assert!(!test_vectors.is_empty(), "EIP-7951 test vector set must not be empty");

        for (index, vector) in
            test_vectors.iter().enumerate().filter(|(_, vector)| !vector.no_benchmark)
        {
            let input = hex::decode(&vector.input).unwrap_or_else(|error| {
                panic!("failed to decode input for {index} ({}): {error}", vector.name)
            });
            let expected = hex::decode(&vector.expected).unwrap_or_else(|error| {
                panic!("failed to decode expected for {index} ({}): {error}", vector.name)
            });

            let outcome = p256_verify(&mut system, &input, PrecompileContext::default())
                .unwrap_or_else(|error| {
                    panic!("precompile call failed for {index} ({}): {error}", vector.name)
                });

            assert_eq!(
                outcome, expected,
                "output mismatch for vector {index} ({}), gas={}",
                vector.name, vector.gas
            );

            if expected.is_empty() {
                assert!(
                    outcome.is_empty(),
                    "failure vector must return empty output for {index} ({})",
                    vector.name
                );
            } else {
                assert_eq!(
                    outcome.len(),
                    32,
                    "success output must be 32 bytes for {index} ({})",
                    vector.name
                );
                assert_eq!(
                    outcome[31], 1,
                    "success output must end with 0x01 for {index} ({})",
                    vector.name
                );
                assert!(
                    outcome[..31].iter().all(|byte| *byte == 0),
                    "success output must be zero-padded for {index} ({})",
                    vector.name
                );
            }
        }
    }

    #[test]
    fn verify_impl_matches_vector_expectations() {
        for (index, vector) in
            load_test_vectors().iter().enumerate().filter(|(_, vector)| !vector.no_benchmark)
        {
            let input = hex::decode(&vector.input).unwrap_or_else(|error| {
                panic!("failed to decode input for {index} ({}): {error}", vector.name)
            });
            let expect_success = !vector.expected.is_empty();

            assert_eq!(
                verify_impl(&input),
                expect_success,
                "verify_impl mismatch for vector {index} ({})",
                vector.name
            );
        }
    }

    #[test]
    fn verify_impl_rejects_non_160_byte_inputs() {
        assert!(!verify_impl(&[0u8; 10]));
        assert!(!verify_impl(&[0u8; 159]));
        assert!(!verify_impl(&[0u8; 161]));
    }
}
