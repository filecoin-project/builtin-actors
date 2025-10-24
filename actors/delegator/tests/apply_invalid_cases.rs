use fil_actor_delegator::{ApplyDelegationsParams, DelegationParam, DelegatorActor, GetStorageRootParams, LookupDelegateParams, Method};
use fil_actors_evm_shared::address::EthAddress;
use fil_actors_runtime::runtime::Primitives;
use fil_actors_runtime::test_utils::{MockRuntime, SYSTEM_ACTOR_CODE_ID};
use fil_actors_runtime::SYSTEM_ACTOR_ADDR;
use fvm_ipld_encoding::ipld_block::IpldBlock;
use fvm_shared::error::ExitCode;
use fvm_shared::MethodNum;

fn new_runtime() -> MockRuntime {
    MockRuntime { receiver: fvm_shared::address::Address::new_id(42), ..Default::default() }
}

fn construct(rt: &MockRuntime) {
    rt.expect_validate_caller_any();
    rt.set_caller(*SYSTEM_ACTOR_CODE_ID, SYSTEM_ACTOR_ADDR);
    rt.call::<DelegatorActor>(Method::Constructor as MethodNum, None).unwrap();
    rt.verify();
}

#[test]
fn empty_list_rejected() {
    let rt = new_runtime();
    construct(&rt);

    rt.expect_validate_caller_any();
    let params = ApplyDelegationsParams { list: vec![] };
    let err = rt
        .call::<DelegatorActor>(
            Method::ApplyDelegations as MethodNum,
            IpldBlock::serialize_dag_cbor(&params).unwrap(),
        )
        .unwrap_err();
    assert_eq!(err.exit_code(), ExitCode::USR_ILLEGAL_ARGUMENT);
}

#[test]
fn invalid_chain_id_rejected() {
    let rt = MockRuntime { receiver: fvm_shared::address::Address::new_id(100), ..Default::default() };
    construct(&rt);

    // chain_id neither 0 nor local
    let params = ApplyDelegationsParams { list: vec![DelegationParam {
        chain_id: 42,
        address: EthAddress::from_id(9),
        nonce: 0,
        y_parity: 0,
        r: [1u8; 32],
        s: [1u8; 32],
    } ]};

    rt.expect_validate_caller_any();
    let err = rt
        .call::<DelegatorActor>(
            Method::ApplyDelegations as MethodNum,
            IpldBlock::serialize_dag_cbor(&params).unwrap(),
        )
        .unwrap_err();
    assert_eq!(err.exit_code(), ExitCode::USR_ILLEGAL_ARGUMENT);
}

#[test]
fn invalid_y_parity_rejected() {
    let rt = new_runtime();
    construct(&rt);

    let params = ApplyDelegationsParams { list: vec![DelegationParam {
        chain_id: 0,
        address: EthAddress::from_id(10),
        nonce: 0,
        y_parity: 2, // only {0,1} allowed
        r: [1u8; 32],
        s: [1u8; 32],
    } ]};

    rt.expect_validate_caller_any();
    let err = rt
        .call::<DelegatorActor>(
            Method::ApplyDelegations as MethodNum,
            IpldBlock::serialize_dag_cbor(&params).unwrap(),
        )
        .unwrap_err();
    assert_eq!(err.exit_code(), ExitCode::USR_ILLEGAL_ARGUMENT);
}

#[test]
fn zero_r_or_s_rejected() {
    let rt = new_runtime();
    construct(&rt);

    // r = 0
    let params_r0 = ApplyDelegationsParams { list: vec![DelegationParam {
        chain_id: 0,
        address: EthAddress::from_id(11),
        nonce: 0,
        y_parity: 0,
        r: [0u8; 32],
        s: [1u8; 32],
    } ]};
    rt.expect_validate_caller_any();
    let err = rt
        .call::<DelegatorActor>(
            Method::ApplyDelegations as MethodNum,
            IpldBlock::serialize_dag_cbor(&params_r0).unwrap(),
        )
        .unwrap_err();
    assert_eq!(err.exit_code(), ExitCode::USR_ILLEGAL_ARGUMENT);

    // s = 0
    let params_s0 = ApplyDelegationsParams { list: vec![DelegationParam {
        chain_id: 0,
        address: EthAddress::from_id(12),
        nonce: 0,
        y_parity: 1,
        r: [1u8; 32],
        s: [0u8; 32],
    } ]};
    rt.expect_validate_caller_any();
    let err = rt
        .call::<DelegatorActor>(
            Method::ApplyDelegations as MethodNum,
            IpldBlock::serialize_dag_cbor(&params_s0).unwrap(),
        )
        .unwrap_err();
    assert_eq!(err.exit_code(), ExitCode::USR_ILLEGAL_ARGUMENT);
}

#[test]
fn high_s_rejected() {
    let rt = new_runtime();
    construct(&rt);

    // s set to max value (> n/2)
    let params = ApplyDelegationsParams { list: vec![DelegationParam {
        chain_id: 0,
        address: EthAddress::from_id(13),
        nonce: 0,
        y_parity: 0,
        r: [1u8; 32],
        s: [0xFFu8; 32],
    } ]};
    rt.expect_validate_caller_any();
    let err = rt
        .call::<DelegatorActor>(
            Method::ApplyDelegations as MethodNum,
            IpldBlock::serialize_dag_cbor(&params).unwrap(),
        )
        .unwrap_err();
    assert_eq!(err.exit_code(), ExitCode::USR_ILLEGAL_ARGUMENT);
}

#[test]
fn signature_recovery_failure_rejected() {
    let rt = MockRuntime { recover_secp_pubkey_fn: Box::new(|_, _| Err(())), ..new_runtime() };
    construct(&rt);

    // Use arbitrary r/s that parse but don't correspond to the digest; y_parity within {0,1}
    let params = ApplyDelegationsParams { list: vec![DelegationParam {
        chain_id: 0,
        address: EthAddress::from_id(14),
        nonce: 0,
        y_parity: 0,
        r: [1u8; 32],
        s: [2u8; 32],
    } ]};
    rt.expect_validate_caller_any();
    let err = rt
        .call::<DelegatorActor>(
            Method::ApplyDelegations as MethodNum,
            IpldBlock::serialize_dag_cbor(&params).unwrap(),
        )
        .unwrap_err();
    assert_eq!(err.exit_code(), ExitCode::USR_ILLEGAL_ARGUMENT);
}

#[test]
fn lookup_none_before_apply_and_get_storage_root_none() {
    let rt = new_runtime();
    construct(&rt);

    // LookupDelegate should be None before any apply
    rt.expect_validate_caller_any();
    let out = rt
        .call::<DelegatorActor>(
            Method::LookupDelegate as MethodNum,
            IpldBlock::serialize_cbor(&LookupDelegateParams { authority: EthAddress::from_id(99) }).unwrap(),
        )
        .unwrap()
        .unwrap()
        .deserialize::<fil_actor_delegator::LookupDelegateReturn>()
        .unwrap();
    assert_eq!(out.delegate, None);

    // GetStorageRoot should be None for untouched authority
    rt.expect_validate_caller_any();
    let out = rt
        .call::<DelegatorActor>(
            Method::GetStorageRoot as MethodNum,
            IpldBlock::serialize_cbor(&GetStorageRootParams { authority: EthAddress::from_id(99) }).unwrap(),
        )
        .unwrap()
        .unwrap()
        .deserialize::<fil_actor_delegator::GetStorageRootReturn>()
        .unwrap();
    assert!(out.root.is_none());
}

#[test]
fn local_chain_id_is_accepted() {
    use k256::ecdsa::{signature::hazmat::PrehashSigner, RecoveryId, Signature as EcdsaSignature, SigningKey, VerifyingKey};
    use rlp::RlpStream;

    // Set a non-zero local chain id and sign with that chain id.
    let rt = MockRuntime { receiver: fvm_shared::address::Address::new_id(77), chain_id: fvm_shared::chainid::ChainID::from(99u64), ..Default::default() };
    construct(&rt);

    let sk = SigningKey::from_bytes(&[9u8; 32].into()).unwrap();
    let vk = VerifyingKey::from(&sk);

    let delegate = EthAddress::from_id(77);
    let mut s = RlpStream::new_list(3);
    s.append(&99u64); // local chain id
    s.append(&delegate.as_ref());
    s.append(&0u64);
    let mut digest = [0u8; 32];
    digest.copy_from_slice(&rt.hash(fvm_shared::crypto::hash::SupportedHashes::Keccak256, &s.out()));
    let sig: EcdsaSignature = sk.sign_prehash(&digest).unwrap();
    let recid = RecoveryId::trial_recovery_from_prehash(&vk, &digest, &sig).unwrap();

    rt.expect_validate_caller_any();
    let params = ApplyDelegationsParams { list: vec![DelegationParam {
        chain_id: 99,
        address: delegate,
        nonce: 0,
        y_parity: recid.to_byte(),
        r: sig.r().to_bytes().into(),
        s: sig.s().to_bytes().into(),
    } ] };
    let ret = rt.call::<DelegatorActor>(
        Method::ApplyDelegations as MethodNum,
        IpldBlock::serialize_dag_cbor(&params).unwrap(),
    );
    assert!(ret.unwrap().is_none());
}
