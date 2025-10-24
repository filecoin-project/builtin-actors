use fil_actor_delegator::{ApplyDelegationsParams, DelegationParam, DelegatorActor, LookupDelegateParams, LookupDelegateReturn, Method};
use fil_actors_evm_shared::address::EthAddress;
use fil_actors_runtime::runtime::Primitives;
use fil_actors_runtime::test_utils::{MockRuntime, SYSTEM_ACTOR_CODE_ID};
use fil_actors_runtime::SYSTEM_ACTOR_ADDR;
use fvm_ipld_encoding::ipld_block::IpldBlock;
use fvm_shared::MethodNum;

fn new_runtime() -> MockRuntime {
    MockRuntime { receiver: fvm_shared::address::Address::new_id(2048), ..Default::default() }
}

fn construct(rt: &MockRuntime) {
    rt.expect_validate_caller_any();
    rt.set_caller(*SYSTEM_ACTOR_CODE_ID, SYSTEM_ACTOR_ADDR);
    rt.call::<DelegatorActor>(Method::Constructor as MethodNum, None).unwrap();
    rt.verify();
}

#[test]
fn apply_multiple_across_authorities_in_one_call() {
    let rt = new_runtime();
    construct(&rt);

    // Create two authorities from secp256k1 keys
    use k256::ecdsa::{signature::hazmat::PrehashSigner, RecoveryId, Signature as EcdsaSignature, SigningKey, VerifyingKey};
    let sk1 = SigningKey::from_bytes(&[7u8; 32].into()).unwrap();
    let vk1 = VerifyingKey::from(&sk1);
    let pk1 = vk1.to_encoded_point(false);
    let (_h1, _len1) = rt.hash_64(fvm_shared::crypto::hash::SupportedHashes::Keccak256, &pk1.as_bytes()[1..]);
    let mut a1b = [0u8; 20];
    a1b.copy_from_slice(&_h1[12..32]);
    let authority1 = EthAddress(a1b);

    let sk2 = SigningKey::from_bytes(&[8u8; 32].into()).unwrap();
    let vk2 = VerifyingKey::from(&sk2);
    let pk2 = vk2.to_encoded_point(false);
    let (_h2, _len2) = rt.hash_64(fvm_shared::crypto::hash::SupportedHashes::Keccak256, &pk2.as_bytes()[1..]);
    let mut a2b = [0u8; 20];
    a2b.copy_from_slice(&_h2[12..32]);
    let authority2 = EthAddress(a2b);

    let delegate1 = EthAddress::from_id(7001);
    let delegate2 = EthAddress::from_id(7002);

    // Build digests and signatures for both tuples (chain_id=0, nonce=0)
    use rlp::RlpStream;
    let mut s1 = RlpStream::new_list(3);
    s1.append(&0u64);
    s1.append(&delegate1.as_ref());
    s1.append(&0u64);
    let mut d1 = [0u8; 32];
    d1.copy_from_slice(&rt.hash(fvm_shared::crypto::hash::SupportedHashes::Keccak256, &s1.out()));
    let sig1: EcdsaSignature = sk1.sign_prehash(&d1).unwrap();
    let recid1 = RecoveryId::trial_recovery_from_prehash(&vk1, &d1, &sig1).unwrap();

    let mut s2 = RlpStream::new_list(3);
    s2.append(&0u64);
    s2.append(&delegate2.as_ref());
    s2.append(&0u64);
    let mut d2 = [0u8; 32];
    d2.copy_from_slice(&rt.hash(fvm_shared::crypto::hash::SupportedHashes::Keccak256, &s2.out()));
    let sig2: EcdsaSignature = sk2.sign_prehash(&d2).unwrap();
    let recid2 = RecoveryId::trial_recovery_from_prehash(&vk2, &d2, &sig2).unwrap();

    // Apply both in one call
    rt.expect_validate_caller_any();
    let params = ApplyDelegationsParams { list: vec![
        DelegationParam { chain_id: 0, address: delegate1, nonce: 0, y_parity: recid1.to_byte(), r: sig1.r().to_bytes().into(), s: sig1.s().to_bytes().into() },
        DelegationParam { chain_id: 0, address: delegate2, nonce: 0, y_parity: recid2.to_byte(), r: sig2.r().to_bytes().into(), s: sig2.s().to_bytes().into() },
    ] };
    let ret = rt.call::<DelegatorActor>(
        Method::ApplyDelegations as MethodNum,
        IpldBlock::serialize_dag_cbor(&params).unwrap(),
    );
    assert!(ret.unwrap().is_none());

    // Both lookups should be populated
    rt.expect_validate_caller_any();
    let out1: LookupDelegateReturn = rt
        .call::<DelegatorActor>(
            Method::LookupDelegate as MethodNum,
            IpldBlock::serialize_cbor(&LookupDelegateParams { authority: authority1 }).unwrap(),
        )
        .unwrap()
        .unwrap()
        .deserialize()
        .unwrap();
    assert_eq!(out1.delegate, Some(delegate1));

    rt.expect_validate_caller_any();
    let out2: LookupDelegateReturn = rt
        .call::<DelegatorActor>(
            Method::LookupDelegate as MethodNum,
            IpldBlock::serialize_cbor(&LookupDelegateParams { authority: authority2 }).unwrap(),
        )
        .unwrap()
        .unwrap()
        .deserialize()
        .unwrap();
    assert_eq!(out2.delegate, Some(delegate2));

    // Re-applying first tuple with same nonce should now fail (nonce bumped)
    rt.expect_validate_caller_any();
    let err = rt
        .call::<DelegatorActor>(
            Method::ApplyDelegations as MethodNum,
            IpldBlock::serialize_dag_cbor(&ApplyDelegationsParams { list: vec![DelegationParam {
                chain_id: 0, address: delegate1, nonce: 0, y_parity: recid1.to_byte(), r: sig1.r().to_bytes().into(), s: sig1.s().to_bytes().into()
            }] }).unwrap(),
        )
        .unwrap_err();
    assert_eq!(err.exit_code(), fvm_shared::error::ExitCode::USR_ILLEGAL_ARGUMENT);
}

