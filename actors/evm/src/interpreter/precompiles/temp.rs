use crate::interpreter::{
    System,
    precompiles::{PrecompileContext, PrecompileResult},
};
use fil_actors_runtime::runtime::Runtime;

// p256 + ecdsa
use p256::ecdsa::{self, Signature, VerifyingKey};
// no need for EncodedPoint type import; we build SEC1 bytes directly

/// RIP-7212 P256VERIFY precompile
///
/// Input (160 bytes, big-endian):
///  - 32: message digest (hash)
///  - 32: r
///  - 32: s
///  - 32: x
///  - 32: y
///
/// Output:
///
/// - On success: 32-byte big-endian integer 1
/// - On failure: empty
pub fn p256_verify<RT: Runtime>(
    _: &mut System<RT>,
    input: &[u8],
    _: PrecompileContext,
) -> PrecompileResult {
    if input.len() != 160 {
        return Ok(Vec::new());
    }

    let hash = &input[0..32];
    let r = &input[32..64];
    let s = &input[64..96];
    let x = &input[96..128];
    let y = &input[128..160];

    // Reject (x,y) == (0,0) explicitly per spec.
    let all_zero = |b: &[u8]| b.iter().all(|&bb| bb == 0);
    if all_zero(x) && all_zero(y) {
        return Ok(Vec::new());
    }

    // Build an uncompressed SEC1 encoded point from (x,y) = 0x04 || X || Y
    let mut sec1 = [0u8; 65];
    sec1[0] = 0x04;
    sec1[1..33].copy_from_slice(x);
    sec1[33..65].copy_from_slice(y);

    // Parse verifying key; this enforces point is on curve and coordinates < p.
    let vk = match VerifyingKey::from_sec1_bytes(&sec1) {
        Ok(v) => v,
        Err(_) => return Ok(Vec::new()),
    };

    // Concatenate r||s into a 64-byte raw signature.
    let mut sig_bytes = [0u8; 64];
    sig_bytes[0..32].copy_from_slice(r);
    sig_bytes[32..64].copy_from_slice(s);
    let sig = match Signature::from_slice(&sig_bytes) {
        Ok(s) => s,
        Err(_) => return Ok(Vec::new()),
    };

    // Verify over pre-hashed message (32 bytes).
    // Use hazmat prehash verifier to avoid re-hashing.
    use ecdsa::signature::hazmat::PrehashVerifier;
    match VerifyingKey::verify_prehash(&vk, hash, &sig) {
        Ok(()) => {
            let mut out = [0u8; 32];
            out[31] = 1;
            Ok(out.to_vec())
        }
        Err(_) => Ok(vec![]),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use fil_actors_runtime::test_utils::MockRuntime;
    use p256::ecdsa::SigningKey;
    use p256::ecdsa::signature::hazmat::PrehashSigner;
    use rand::rngs::StdRng;
    use rand::{RngCore, SeedableRng};

    #[test]
    fn verify_valid_signature_returns_one() {
        let rt = MockRuntime::default();
        rt.in_call.replace(true);
        let mut sys = crate::interpreter::System::create(&rt).unwrap();

        let mut rng = StdRng::seed_from_u64(42);
        let sk = SigningKey::random(&mut rng);
        let vk = VerifyingKey::from(&sk);

        let mut hash = [0u8; 32];
        rng.fill_bytes(&mut hash);

        let sig: p256::ecdsa::Signature = PrehashSigner::sign_prehash(&sk, &hash).unwrap();

        let pk = vk.to_encoded_point(false);
        let (x, y) = (pk.x().unwrap(), pk.y().unwrap());
        let (r_bytes, s_bytes) = (sig.r().to_bytes(), sig.s().to_bytes());

        let mut input = Vec::with_capacity(160);
        input.extend_from_slice(&hash);
        input.extend_from_slice(&r_bytes);
        input.extend_from_slice(&s_bytes);
        input.extend_from_slice(x);
        input.extend_from_slice(y);

        let out = p256_verify::<MockRuntime>(&mut sys, &input, default_ctx()).unwrap();
        assert_eq!(out.len(), 32);
        assert_eq!(out[31], 1);
        assert!(out[..31].iter().all(|b| *b == 0));
    }

    #[test]
    fn verify_wrong_hash_returns_empty() {
        let rt = MockRuntime::default();
        rt.in_call.replace(true);
        let mut sys = crate::interpreter::System::create(&rt).unwrap();

        let mut rng = StdRng::seed_from_u64(1337);
        let sk = SigningKey::random(&mut rng);
        let vk = VerifyingKey::from(&sk);

        let mut hash = [0u8; 32];
        rng.fill_bytes(&mut hash);
        let sig: p256::ecdsa::Signature = PrehashSigner::sign_prehash(&sk, &hash).unwrap();

        // Mutate the hash to make it invalid.
        hash[0] ^= 0x01;

        let pk = vk.to_encoded_point(false);
        let (x, y) = (pk.x().unwrap(), pk.y().unwrap());
        let (r_bytes, s_bytes) = (sig.r().to_bytes(), sig.s().to_bytes());

        let mut input = Vec::with_capacity(160);
        input.extend_from_slice(&hash);
        input.extend_from_slice(&r_bytes);
        input.extend_from_slice(&s_bytes);
        input.extend_from_slice(x);
        input.extend_from_slice(y);

        let out = p256_verify::<MockRuntime>(&mut sys, &input, default_ctx()).unwrap();
        assert!(out.is_empty());
    }

    #[test]
    fn invalid_input_size() {
        let rt = MockRuntime::default();
        rt.in_call.replace(true);
        let mut sys = crate::interpreter::System::create(&rt).unwrap();

        let out = p256_verify::<MockRuntime>(&mut sys, &[0u8; 10], default_ctx()).unwrap();
        assert!(out.is_empty());
    }

    #[test]
    fn invalid_r_zero() {
        let rt = MockRuntime::default();
        rt.in_call.replace(true);
        let mut sys = crate::interpreter::System::create(&rt).unwrap();

        let mut rng = StdRng::seed_from_u64(99);
        let sk = SigningKey::random(&mut rng);
        let vk = VerifyingKey::from(&sk);

        let mut hash = [0u8; 32];
        rng.fill_bytes(&mut hash);
        let sig: p256::ecdsa::Signature = PrehashSigner::sign_prehash(&sk, &hash).unwrap();

        let pk = vk.to_encoded_point(false);
        let (x, y) = (pk.x().unwrap(), pk.y().unwrap());

        let mut input = Vec::with_capacity(160);
        input.extend_from_slice(&hash);
        input.extend_from_slice(&[0u8; 32]); // r = 0
        input.extend_from_slice(&sig.s().to_bytes());
        input.extend_from_slice(x);
        input.extend_from_slice(y);

        let out = p256_verify::<MockRuntime>(&mut sys, &input, default_ctx()).unwrap();
        assert!(out.is_empty());
    }

    #[test]
    fn invalid_s_zero() {
        let rt = MockRuntime::default();
        rt.in_call.replace(true);
        let mut sys = crate::interpreter::System::create(&rt).unwrap();

        let mut rng = StdRng::seed_from_u64(100);
        let sk = SigningKey::random(&mut rng);
        let vk = VerifyingKey::from(&sk);

        let mut hash = [0u8; 32];
        rng.fill_bytes(&mut hash);
        let sig: p256::ecdsa::Signature = PrehashSigner::sign_prehash(&sk, &hash).unwrap();

        let pk = vk.to_encoded_point(false);
        let (x, y) = (pk.x().unwrap(), pk.y().unwrap());

        let mut input = Vec::with_capacity(160);
        input.extend_from_slice(&hash);
        input.extend_from_slice(&sig.r().to_bytes());
        input.extend_from_slice(&[0u8; 32]); // s = 0
        input.extend_from_slice(x);
        input.extend_from_slice(y);

        let out = p256_verify::<MockRuntime>(&mut sys, &input, default_ctx()).unwrap();
        assert!(out.is_empty());
    }

    #[test]
    fn invalid_r_ge_n() {
        let rt = MockRuntime::default();
        rt.in_call.replace(true);
        let mut sys = crate::interpreter::System::create(&rt).unwrap();

        let mut rng = StdRng::seed_from_u64(101);
        let sk = SigningKey::random(&mut rng);
        let vk = VerifyingKey::from(&sk);

        let mut hash = [0u8; 32];
        rng.fill_bytes(&mut hash);
        let sig: p256::ecdsa::Signature = PrehashSigner::sign_prehash(&sk, &hash).unwrap();

        // r >= n: use 0xff..ff as a guaranteed out-of-range value
        let r_bad = [0xffu8; 32];

        let pk = vk.to_encoded_point(false);
        let (x, y) = (pk.x().unwrap(), pk.y().unwrap());

        let mut input = Vec::with_capacity(160);
        input.extend_from_slice(&hash);
        input.extend_from_slice(&r_bad);
        input.extend_from_slice(&sig.s().to_bytes());
        input.extend_from_slice(x);
        input.extend_from_slice(y);

        let out = p256_verify::<MockRuntime>(&mut sys, &input, default_ctx()).unwrap();
        assert!(out.is_empty());
    }

    #[test]
    fn invalid_s_ge_n() {
        let rt = MockRuntime::default();
        rt.in_call.replace(true);
        let mut sys = crate::interpreter::System::create(&rt).unwrap();

        let mut rng = StdRng::seed_from_u64(102);
        let sk = SigningKey::random(&mut rng);
        let vk = VerifyingKey::from(&sk);

        let mut hash = [0u8; 32];
        rng.fill_bytes(&mut hash);
        let sig: p256::ecdsa::Signature = PrehashSigner::sign_prehash(&sk, &hash).unwrap();

        let s_bad = [0xffu8; 32];

        let pk = vk.to_encoded_point(false);
        let (x, y) = (pk.x().unwrap(), pk.y().unwrap());

        let mut input = Vec::with_capacity(160);
        input.extend_from_slice(&hash);
        input.extend_from_slice(&sig.r().to_bytes());
        input.extend_from_slice(&s_bad);
        input.extend_from_slice(x);
        input.extend_from_slice(y);

        let out = p256_verify::<MockRuntime>(&mut sys, &input, default_ctx()).unwrap();
        assert!(out.is_empty());
    }

    #[test]
    fn invalid_pubkey_zero_zero() {
        let rt = MockRuntime::default();
        rt.in_call.replace(true);
        let mut sys = crate::interpreter::System::create(&rt).unwrap();

        let mut rng = StdRng::seed_from_u64(103);
        let sk = SigningKey::random(&mut rng);

        let mut hash = [0u8; 32];
        rng.fill_bytes(&mut hash);
        let sig: p256::ecdsa::Signature = PrehashSigner::sign_prehash(&sk, &hash).unwrap();

        let mut input = Vec::with_capacity(160);
        input.extend_from_slice(&hash);
        input.extend_from_slice(&sig.r().to_bytes());
        input.extend_from_slice(&sig.s().to_bytes());
        input.extend_from_slice(&[0u8; 32]); // x = 0
        input.extend_from_slice(&[0u8; 32]); // y = 0

        let out = p256_verify::<MockRuntime>(&mut sys, &input, default_ctx()).unwrap();
        assert!(out.is_empty());
    }

    #[test]
    fn invalid_pubkey_off_curve() {
        let rt = MockRuntime::default();
        rt.in_call.replace(true);
        let mut sys = crate::interpreter::System::create(&rt).unwrap();

        let mut rng = StdRng::seed_from_u64(104);
        let sk = SigningKey::random(&mut rng);

        let mut hash = [0u8; 32];
        rng.fill_bytes(&mut hash);
        let sig: p256::ecdsa::Signature = PrehashSigner::sign_prehash(&sk, &hash).unwrap();

        let mut x = [0u8; 32];
        x[31] = 1; // x=1
        let y = [0u8; 32]; // y=0

        let mut input = Vec::with_capacity(160);
        input.extend_from_slice(&hash);
        input.extend_from_slice(&sig.r().to_bytes());
        input.extend_from_slice(&sig.s().to_bytes());
        input.extend_from_slice(&x);
        input.extend_from_slice(&y);

        let out = p256_verify::<MockRuntime>(&mut sys, &input, default_ctx()).unwrap();
        assert!(out.is_empty());
    }

    #[test]
    fn invalid_pubkey_coords_ge_p() {
        let rt = MockRuntime::default();
        rt.in_call.replace(true);
        let mut sys = crate::interpreter::System::create(&rt).unwrap();

        let mut rng = StdRng::seed_from_u64(105);
        let sk = SigningKey::random(&mut rng);

        let mut hash = [0u8; 32];
        rng.fill_bytes(&mut hash);
        let sig: p256::ecdsa::Signature = PrehashSigner::sign_prehash(&sk, &hash).unwrap();

        // x and/or y >= p: use 0xff..ff to ensure invalid
        let x = [0xffu8; 32];
        let y = [0xffu8; 32];

        let mut input = Vec::with_capacity(160);
        input.extend_from_slice(&hash);
        input.extend_from_slice(&sig.r().to_bytes());
        input.extend_from_slice(&sig.s().to_bytes());
        input.extend_from_slice(&x);
        input.extend_from_slice(&y);

        let out = p256_verify::<MockRuntime>(&mut sys, &input, default_ctx()).unwrap();
        assert!(out.is_empty());
    }

    #[test]
    fn revm_vectors() {
        use hex::FromHex;
        let rt = MockRuntime::default();
        rt.in_call.replace(true);
        let mut sys = crate::interpreter::System::create(&rt).unwrap();

        // Success vectors
        let ok = [
            "4cee90eb86eaa050036147a12d49004b6b9c72bd725d39d4785011fe190f0b4da73bd4903f0ce3b639bbbf6e8e80d16931ff4bcf5993d58468e8fb19086e8cac36dbcd03009df8c59286b162af3bd7fcc0450c9aa81be5d10d312af6c66b1d604aebd3099c618202fcfe16ae7770b0c49ab5eadf74b754204a3bb6060e44eff37618b065f9832de4ca6ca971a7a1adc826d0f7c00181a5fb2ddf79ae00b4e10e",
            "3fec5769b5cf4e310a7d150508e82fb8e3eda1c2c94c61492d3bd8aea99e06c9e22466e928fdccef0de49e3503d2657d00494a00e764fd437bdafa05f5922b1fbbb77c6817ccf50748419477e843d5bac67e6a70e97dde5a57e0c983b777e1ad31a80482dadf89de6302b1988c82c29544c9c07bb910596158f6062517eb089a2f54c9a0f348752950094d3228d3b940258c75fe2a413cb70baa21dc2e352fc5",
            "e775723953ead4a90411a02908fd1a629db584bc600664c609061f221ef6bf7c440066c8626b49daaa7bf2bcc0b74be4f7a1e3dcf0e869f1542fe821498cbf2de73ad398194129f635de4424a07ca715838aefe8fe69d1a391cfa70470795a80dd056866e6e1125aff94413921880c437c9e2570a28ced7267c8beef7e9b2d8d1547d76dfcf4bee592f5fefe10ddfb6aeb0991c5b9dbbee6ec80d11b17c0eb1a",
            "b5a77e7a90aa14e0bf5f337f06f597148676424fae26e175c6e5621c34351955289f319789da424845c9eac935245fcddd805950e2f02506d09be7e411199556d262144475b1fa46ad85250728c600c53dfd10f8b3f4adf140e27241aec3c2da3a81046703fccf468b48b145f939efdbb96c3786db712b3113bb2488ef286cdcef8afe82d200a5bb36b5462166e8ce77f2d831a52ef2135b2af188110beaefb1",
            "858b991cfd78f16537fe6d1f4afd10273384db08bdfc843562a22b0626766686f6aec8247599f40bfe01bec0e0ecf17b4319559022d4d9bf007fe929943004eb4866760dedf31b7c691f5ce665f8aae0bda895c23595c834fecc2390a5bcc203b04afcacbb4280713287a2d0c37e23f7513fab898f2c1fefa00ec09a924c335d9b629f1d4fb71901c3e59611afbfea354d101324e894c788d1c01f00b3c251b2",
        ];
        for hexstr in ok.iter() {
            let input = Vec::from_hex(hexstr).unwrap();
            let out = super::p256_verify::<MockRuntime>(&mut sys, &input, default_ctx()).unwrap();
            assert_eq!(out.len(), 32);
            assert_eq!(out[31], 1);
        }

        // Failure vectors -> empty output
        let fail = [
            "3cee90eb86eaa050036147a12d49004b6b9c72bd725d39d4785011fe190f0b4da73bd4903f0ce3b639bbbf6e8e80d16931ff4bcf5993d58468e8fb19086e8cac36dbcd03009df8c59286b162af3bd7fcc0450c9aa81be5d10d312af6c66b1d604aebd3099c618202fcfe16ae7770b0c49ab5eadf74b754204a3bb6060e44eff37618b065f9832de4ca6ca971a7a1adc826d0f7c00181a5fb2ddf79ae00b4e10e",
            "afec5769b5cf4e310a7d150508e82fb8e3eda1c2c94c61492d3bd8aea99e06c9e22466e928fdccef0de49e3503d2657d00494a00e764fd437bdafa05f5922b1fbbb77c6817ccf50748419477e843d5bac67e6a70e97dde5a57e0c983b777e1ad31a80482dadf89de6302b1988c82c29544c9c07bb910596158f6062517eb089a2f54c9a0f348752950094d3228d3b940258c75fe2a413cb70baa21dc2e352fc5",
            "f775723953ead4a90411a02908fd1a629db584bc600664c609061f221ef6bf7c440066c8626b49daaa7bf2bcc0b74be4f7a1e3dcf0e869f1542fe821498cbf2de73ad398194129f635de4424a07ca715838aefe8fe69d1a391cfa70470795a80dd056866e6e1125aff94413921880c437c9e2570a28ced7267c8beef7e9b2d8d1547d76dfcf4bee592f5fefe10ddfb6aeb0991c5b9dbbee6ec80d11b17c0eb1a",
            "c5a77e7a90aa14e0bf5f337f06f597148676424fae26e175c6e5621c34351955289f319789da424845c9eac935245fcddd805950e2f02506d09be7e411199556d262144475b1fa46ad85250728c600c53dfd10f8b3f4adf140e27241aec3c2da3a81046703fccf468b48b145f939efdbb96c3786db712b3113bb2488ef286cdcef8afe82d200a5bb36b5462166e8ce77f2d831a52ef2135b2af188110beaefb1",
            "958b991cfd78f16537fe6d1f4afd10273384db08bdfc843562a22b0626766686f6aec8247599f40bfe01bec0e0ecf17b4319559022d4d9bf007fe929943004eb4866760dedf31b7c691f5ce665f8aae0bda895c23595c834fecc2390a5bcc203b04afcacbb4280713287a2d0c37e23f7513fab898f2c1fefa00ec09a924c335d9b629f1d4fb71901c3e59611afbfea354d101324e894c788d1c01f00b3c251b2",
            // short/long input
            "4cee90eb86eaa050036147a12d49004b6a",
            "4cee90eb86eaa050036147a12d49004b6a958b991cfd78f16537fe6d1f4afd10273384db08bdfc843562a22b0626766686f6aec8247599f40bfe01bec0e0ecf17b4319559022d4d9bf007fe929943004eb4866760dedf319",
            "4cee90eb86eaa050036147a12d49004b6b9c72bd725d39d4785011fe190f0b4da73bd4903f0ce3b639bbbf6e8e80d16931ff4bcf5993d58468e8fb19086e8cac36dbcd03009df8c59286b162af3bd7fcc0450c9aa81be5d10d312af6c66b1d604aebd3099c618202fcfe16ae7770b0c49ab5eadf74b754204a3bb6060e44eff37618b065f9832de4ca6ca971a7a1adc826d0f7c00181a5fb2ddf79ae00b4e10e00",
            // invalid sig
            "4cee90eb86eaa050036147a12d49004b6b9c72bd725d39d4785011fe190f0b4dffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff4aebd3099c618202fcfe16ae7770b0c49ab5eadf74b754204a3bb6060e44eff37618b065f9832de4ca6ca971a7a1adc826d0f7c00181a5fb2ddf79ae00b4e10e",
            // invalid pubkey (zero)
            "4cee90eb86eaa050036147a12d49004b6b9c72bd725d39d4785011fe190f0b4da73bd4903f0ce3b639bbbf6e8e80d16931ff4bcf5993d58468e8fb19086e8cac36dbcd03009df8c59286b162af3bd7fcc0450c9aa81be5d10d312af6c66b1d6000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000",
        ];
        for hexstr in fail.iter() {
            let input = Vec::from_hex(hexstr).unwrap();
            let out = super::p256_verify::<MockRuntime>(&mut sys, &input, default_ctx()).unwrap();
            assert!(out.is_empty());
        }
    }

    fn default_ctx() -> PrecompileContext {
        PrecompileContext {
            call_type: crate::interpreter::CallKind::StaticCall,
            gas: 0u8.into(),
            value: 0u8.into(),
        }
    }
}