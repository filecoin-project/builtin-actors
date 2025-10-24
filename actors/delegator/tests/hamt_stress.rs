use fil_actor_delegator::{ApplyDelegationsParams, DelegationParam, DelegatorActor, LookupDelegateParams, LookupDelegateReturn, Method};
use fil_actors_evm_shared::address::EthAddress;
use fil_actors_runtime::runtime::Primitives;
use fil_actors_runtime::test_utils::{MockRuntime, SYSTEM_ACTOR_CODE_ID};
use fil_actors_runtime::SYSTEM_ACTOR_ADDR;
use fvm_ipld_encoding::ipld_block::IpldBlock;
use fvm_shared::MethodNum;

fn construct() -> MockRuntime {
    let rt = MockRuntime { receiver: fvm_shared::address::Address::new_id(6000), ..Default::default() };
    rt.expect_validate_caller_any();
    rt.set_caller(*SYSTEM_ACTOR_CODE_ID, SYSTEM_ACTOR_ADDR);
    rt.call::<DelegatorActor>(Method::Constructor as MethodNum, None).unwrap();
    rt.verify();
    rt
}

#[test]
fn apply_many_mappings_and_verify_all() {
    use k256::ecdsa::{signature::hazmat::PrehashSigner, RecoveryId, Signature as EcdsaSignature, SigningKey, VerifyingKey};
    use rlp::RlpStream;

    let rt = construct();

    const N: usize = 24;
    let mut list: Vec<DelegationParam> = Vec::with_capacity(N);
    let mut authorities: Vec<EthAddress> = Vec::with_capacity(N);
    let mut delegates: Vec<EthAddress> = Vec::with_capacity(N);

    for i in 0..N {
        let sk = SigningKey::from_bytes(&[i as u8 + 1; 32].into()).unwrap();
        let vk = VerifyingKey::from(&sk);
        let pubkey = vk.to_encoded_point(false);
        let (keccak, _) = rt.hash_64(fvm_shared::crypto::hash::SupportedHashes::Keccak256, &pubkey.as_bytes()[1..]);
        let mut auth_bytes = [0u8; 20];
        auth_bytes.copy_from_slice(&keccak[12..32]);
        let authority = EthAddress(auth_bytes);
        let delegate = EthAddress::from_id(10_000 + i as u64);

        // digest(chain_id=0, address, nonce=0)
        let mut s = RlpStream::new_list(3);
        s.append(&0u64);
        s.append(&delegate.as_ref());
        s.append(&0u64);
        let mut d = [0u8; 32];
        d.copy_from_slice(&rt.hash(fvm_shared::crypto::hash::SupportedHashes::Keccak256, &s.out()));
        let sig: EcdsaSignature = sk.sign_prehash(&d).unwrap();
        let recid = RecoveryId::trial_recovery_from_prehash(&vk, &d, &sig).unwrap();

        list.push(DelegationParam { chain_id: 0, address: delegate, nonce: 0, y_parity: recid.to_byte(), r: sig.r().to_bytes().into(), s: sig.s().to_bytes().into() });
        authorities.push(authority);
        delegates.push(delegate);
    }

    rt.expect_validate_caller_any();
    let ret = rt.call::<DelegatorActor>(
        Method::ApplyDelegations as MethodNum,
        IpldBlock::serialize_dag_cbor(&ApplyDelegationsParams { list }).unwrap(),
    );
    assert!(ret.unwrap().is_none());

    // Verify a subset and edges
    for idx in [0, N / 2, N - 1] {
        rt.expect_validate_caller_any();
        let out: LookupDelegateReturn = rt
            .call::<DelegatorActor>(
                Method::LookupDelegate as MethodNum,
                IpldBlock::serialize_cbor(&LookupDelegateParams { authority: authorities[idx] }).unwrap(),
            )
            .unwrap()
            .unwrap()
            .deserialize()
            .unwrap();
        assert_eq!(out.delegate, Some(delegates[idx]));
    }
}

