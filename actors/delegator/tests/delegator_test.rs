use cid::Cid;
use fil_actor_delegator::{DelegatorActor, Method, ApplyDelegationsParams, DelegationParam, LookupDelegateParams, LookupDelegateReturn, GetStorageRootParams, GetStorageRootReturn, PutStorageRootParams};
use fil_actors_evm_shared::address::EthAddress;
use fil_actors_runtime::test_utils::{MockRuntime, SYSTEM_ACTOR_CODE_ID, EVM_ACTOR_CODE_ID};
use fil_actors_runtime::SYSTEM_ACTOR_ADDR;
use fil_actors_runtime::runtime::Primitives;
use fvm_ipld_encoding::ipld_block::IpldBlock;
use fvm_shared::MethodNum;
use fvm_shared::error::ExitCode;

fn new_runtime() -> MockRuntime {
    // Receiver can be any ID for tests
    let rt = MockRuntime { receiver: fvm_shared::address::Address::new_id(1024), ..Default::default() };
    rt
}

#[test]
fn apply_and_lookup_mapping_with_recovery_override() {
    let rt = new_runtime();
    // constructor
    rt.expect_validate_caller_any();
    rt.set_caller(*SYSTEM_ACTOR_CODE_ID, SYSTEM_ACTOR_ADDR);
    rt.call::<DelegatorActor>(Method::Constructor as MethodNum, None).unwrap();
    rt.verify();

    // Create a keypair and compute authority address from pubkey
    use k256::ecdsa::{SigningKey, VerifyingKey, signature::hazmat::PrehashSigner, Signature as EcdsaSignature, RecoveryId};
    let sk = SigningKey::from_bytes(&[3u8; 32].into()).unwrap();
    let vk = VerifyingKey::from(&sk);
    let pubkey = vk.to_encoded_point(false);
    let (_k, _len) = rt.hash_64(fvm_shared::crypto::hash::SupportedHashes::Keccak256, &pubkey.as_bytes()[1..]);
    let mut auth_bytes = [0u8; 20];
    auth_bytes.copy_from_slice(&_k[12..32]);
    let authority = EthAddress(auth_bytes);

    // Delegate address (EVM actor-style delegated f4)
    let delegate = EthAddress::from_id(2001);

    // Build digest and sign it
    use rlp::RlpStream;
    let mut s = RlpStream::new_list(3);
    s.append(&0u64);
    s.append(&EthAddress::from_id(2001).as_ref());
    s.append(&0u64);
    let rlp_bytes = s.out().to_vec();
    let mut digest = [0u8; 32];
    let h = rt.hash(fvm_shared::crypto::hash::SupportedHashes::Keccak256, &rlp_bytes);
    digest.copy_from_slice(&h);
    let sig: EcdsaSignature = sk.sign_prehash(&digest).unwrap();
    let recid = RecoveryId::trial_recovery_from_prehash(&vk, &digest, &sig).unwrap();

    // Apply single delegation
    rt.expect_validate_caller_any();
    let params = ApplyDelegationsParams { list: vec![DelegationParam {
        chain_id: 0,
        address: delegate,
        nonce: 0,
        y_parity: recid.to_byte(),
        r: sig.r().to_bytes().into(),
        s: sig.s().to_bytes().into(),
    } ]};
    let ret = rt.call::<DelegatorActor>(
        Method::ApplyDelegations as MethodNum,
        IpldBlock::serialize_dag_cbor(&params).unwrap(),
    );
    assert!(ret.unwrap().is_none());
    rt.verify();

    // Lookup should return delegate
    rt.expect_validate_caller_any();
    let out = rt
        .call::<DelegatorActor>(
            Method::LookupDelegate as MethodNum,
            IpldBlock::serialize_cbor(&LookupDelegateParams { authority }).unwrap(),
        )
        .unwrap()
        .unwrap()
        .deserialize()
        .unwrap();
    let out: LookupDelegateReturn = out;
    assert_eq!(out.delegate, Some(delegate));

    // Applying again with same nonce should error
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
fn storage_root_get_put_permissions() {
    let rt = new_runtime();
    // constructor
    rt.expect_validate_caller_any();
    rt.set_caller(*SYSTEM_ACTOR_CODE_ID, SYSTEM_ACTOR_ADDR);
    rt.call::<DelegatorActor>(Method::Constructor as MethodNum, None).unwrap();
    rt.verify();

    let authority = EthAddress::from_id(3001);
    let fake_cid = Cid::try_from("baeaikaia").unwrap();

    // Non-EVM caller should be forbidden
    rt.expect_validate_caller_any();
    let err = rt.call::<DelegatorActor>(
        Method::PutStorageRoot as MethodNum,
        fvm_ipld_encoding::ipld_block::IpldBlock::serialize_dag_cbor(&PutStorageRootParams { authority, root: fake_cid }).unwrap(),
    ).unwrap_err();
    assert_eq!(err.exit_code(), ExitCode::USR_FORBIDDEN);

    // EVM caller allowed
    rt.expect_validate_caller_any();
    rt.set_caller(*EVM_ACTOR_CODE_ID, SYSTEM_ACTOR_ADDR);
    let res = rt.call::<DelegatorActor>(
        Method::PutStorageRoot as MethodNum,
        fvm_ipld_encoding::ipld_block::IpldBlock::serialize_dag_cbor(&PutStorageRootParams { authority, root: fake_cid }).unwrap(),
    );
    assert!(res.unwrap().is_none());

    // GetStorageRoot returns the stored CID
    rt.expect_validate_caller_any();
    let got: GetStorageRootReturn = rt.call::<DelegatorActor>(
        Method::GetStorageRoot as MethodNum,
        fvm_ipld_encoding::ipld_block::IpldBlock::serialize_cbor(&GetStorageRootParams { authority }).unwrap(),
    ).unwrap().unwrap().deserialize().unwrap();
    assert_eq!(got.root, Some(fake_cid));
}
