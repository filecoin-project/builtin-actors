use fil_actor_delegator::{ApplyDelegationsParams, DelegationParam, DelegatorActor, LookupDelegateParams, LookupDelegateReturn, Method};
use fil_actors_evm_shared::address::EthAddress;
use fil_actors_runtime::runtime::Primitives;
use fil_actors_runtime::test_utils::{MockRuntime, SYSTEM_ACTOR_CODE_ID};
use fil_actors_runtime::SYSTEM_ACTOR_ADDR;
use fvm_ipld_encoding::ipld_block::IpldBlock;
use fvm_shared::error::ExitCode;
use fvm_shared::MethodNum;

fn new_rt() -> MockRuntime { MockRuntime { receiver: fvm_shared::address::Address::new_id(9001), ..Default::default() } }

fn construct(rt: &MockRuntime) {
    rt.expect_validate_caller_any();
    rt.set_caller(*SYSTEM_ACTOR_CODE_ID, SYSTEM_ACTOR_ADDR);
    rt.call::<DelegatorActor>(Method::Constructor as MethodNum, None).unwrap();
    rt.verify();
}

#[test]
fn nonce_increments_across_messages() {
    use k256::ecdsa::{signature::hazmat::PrehashSigner, RecoveryId, Signature as EcdsaSignature, SigningKey, VerifyingKey};
    use rlp::RlpStream;

    let rt = new_rt();
    construct(&rt);

    let sk = SigningKey::from_bytes(&[0x55u8; 32].into()).unwrap();
    let vk = VerifyingKey::from(&sk);
    let pubkey = vk.to_encoded_point(false);
    let (_k, _l) = rt.hash_64(fvm_shared::crypto::hash::SupportedHashes::Keccak256, &pubkey.as_bytes()[1..]);
    let mut ab = [0u8; 20];
    ab.copy_from_slice(&_k[12..32]);
    let authority = EthAddress(ab);
    let delegate = EthAddress::from_id(42);

    // nonce 0
    let mut s0 = RlpStream::new_list(3);
    s0.append(&0u64);
    s0.append(&delegate.as_ref());
    s0.append(&0u64);
    let mut d0 = [0u8; 32];
    d0.copy_from_slice(&rt.hash(fvm_shared::crypto::hash::SupportedHashes::Keccak256, &s0.out()));
    let sig0: EcdsaSignature = sk.sign_prehash(&d0).unwrap();
    let recid0 = RecoveryId::trial_recovery_from_prehash(&vk, &d0, &sig0).unwrap();

    rt.expect_validate_caller_any();
    let ret = rt.call::<DelegatorActor>(
        Method::ApplyDelegations as MethodNum,
        IpldBlock::serialize_dag_cbor(&ApplyDelegationsParams { list: vec![DelegationParam { chain_id: 0, address: delegate, nonce: 0, y_parity: recid0.to_byte(), r: sig0.r().to_bytes().into(), s: sig0.s().to_bytes().into() }] }).unwrap(),
    );
    assert!(ret.unwrap().is_none());

    // nonce 1
    let mut s1 = RlpStream::new_list(3);
    s1.append(&0u64);
    s1.append(&delegate.as_ref());
    s1.append(&1u64);
    let mut d1 = [0u8; 32];
    d1.copy_from_slice(&rt.hash(fvm_shared::crypto::hash::SupportedHashes::Keccak256, &s1.out()));
    let sig1: EcdsaSignature = sk.sign_prehash(&d1).unwrap();
    let recid1 = RecoveryId::trial_recovery_from_prehash(&vk, &d1, &sig1).unwrap();

    rt.expect_validate_caller_any();
    let ret2 = rt.call::<DelegatorActor>(
        Method::ApplyDelegations as MethodNum,
        IpldBlock::serialize_dag_cbor(&ApplyDelegationsParams { list: vec![DelegationParam { chain_id: 0, address: delegate, nonce: 1, y_parity: recid1.to_byte(), r: sig1.r().to_bytes().into(), s: sig1.s().to_bytes().into() }] }).unwrap(),
    );
    assert!(ret2.unwrap().is_none());

    // Lookup still returns delegate
    rt.expect_validate_caller_any();
    let out: LookupDelegateReturn = rt.call::<DelegatorActor>(
        Method::LookupDelegate as MethodNum,
        IpldBlock::serialize_cbor(&LookupDelegateParams { authority }).unwrap(),
    ).unwrap().unwrap().deserialize().unwrap();
    assert_eq!(out.delegate, Some(delegate));
}

#[test]
fn duplicate_same_authority_tuples_semantics() {
    use k256::ecdsa::{signature::hazmat::PrehashSigner, RecoveryId, Signature as EcdsaSignature, SigningKey, VerifyingKey};
    use rlp::RlpStream;
    let rt = new_rt();
    construct(&rt);

    let sk = SigningKey::from_bytes(&[0x77u8; 32].into()).unwrap();
    let vk = VerifyingKey::from(&sk);
    let delegate = EthAddress::from_id(64);

    let mut s0 = RlpStream::new_list(3);
    s0.append(&0u64);
    s0.append(&delegate.as_ref());
    s0.append(&0u64);
    let mut d0 = [0u8; 32]; d0.copy_from_slice(&rt.hash(fvm_shared::crypto::hash::SupportedHashes::Keccak256, &s0.out()));
    let sig0: EcdsaSignature = sk.sign_prehash(&d0).unwrap();
    let recid0 = RecoveryId::trial_recovery_from_prehash(&vk, &d0, &sig0).unwrap();

    let mut s1 = RlpStream::new_list(3);
    s1.append(&0u64);
    s1.append(&delegate.as_ref());
    s1.append(&1u64);
    let mut d1 = [0u8; 32]; d1.copy_from_slice(&rt.hash(fvm_shared::crypto::hash::SupportedHashes::Keccak256, &s1.out()));
    let sig1: EcdsaSignature = sk.sign_prehash(&d1).unwrap();
    let recid1 = RecoveryId::trial_recovery_from_prehash(&vk, &d1, &sig1).unwrap();

    // Success case: two tuples for same authority in one message with nonces 0 then 1 should succeed.
    rt.expect_validate_caller_any();
    let ok = rt.call::<DelegatorActor>(
        Method::ApplyDelegations as MethodNum,
        IpldBlock::serialize_dag_cbor(&ApplyDelegationsParams { list: vec![
            DelegationParam { chain_id: 0, address: delegate, nonce: 0, y_parity: recid0.to_byte(), r: sig0.r().to_bytes().into(), s: sig0.s().to_bytes().into() },
            DelegationParam { chain_id: 0, address: delegate, nonce: 1, y_parity: recid1.to_byte(), r: sig1.r().to_bytes().into(), s: sig1.s().to_bytes().into() },
        ]}).unwrap(),
    );
    assert!(ok.unwrap().is_none());

    // Failure case: two tuples both with nonce=0 should fail on the second.
    rt.expect_validate_caller_any();
    let err = rt.call::<DelegatorActor>(
        Method::ApplyDelegations as MethodNum,
        IpldBlock::serialize_dag_cbor(&ApplyDelegationsParams { list: vec![
            DelegationParam { chain_id: 0, address: delegate, nonce: 0, y_parity: recid0.to_byte(), r: sig0.r().to_bytes().into(), s: sig0.s().to_bytes().into() },
            DelegationParam { chain_id: 0, address: delegate, nonce: 0, y_parity: recid0.to_byte(), r: sig0.r().to_bytes().into(), s: sig0.s().to_bytes().into() },
        ]}).unwrap(),
    ).unwrap_err();
    assert_eq!(err.exit_code(), ExitCode::USR_ILLEGAL_ARGUMENT);

    // Failure case: out-of-order nonces (1 then 0) should fail on the first tuple
    // because expected current nonce is 0 but provided is 1.
    rt.expect_validate_caller_any();
    let err2 = rt.call::<DelegatorActor>(
        Method::ApplyDelegations as MethodNum,
        IpldBlock::serialize_dag_cbor(&ApplyDelegationsParams { list: vec![
            DelegationParam { chain_id: 0, address: delegate, nonce: 1, y_parity: recid1.to_byte(), r: sig1.r().to_bytes().into(), s: sig1.s().to_bytes().into() },
            DelegationParam { chain_id: 0, address: delegate, nonce: 0, y_parity: recid0.to_byte(), r: sig0.r().to_bytes().into(), s: sig0.s().to_bytes().into() },
        ]}).unwrap(),
    ).unwrap_err();
    assert_eq!(err2.exit_code(), ExitCode::USR_ILLEGAL_ARGUMENT);
}
