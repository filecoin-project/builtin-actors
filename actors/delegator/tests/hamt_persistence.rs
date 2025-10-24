use fil_actor_delegator::{ApplyDelegationsParams, DelegationParam, DelegatorActor, Method, State};
use fil_actors_evm_shared::address::EthAddress;
use fil_actors_runtime::runtime::{Primitives, Runtime};
use fil_actors_runtime::test_utils::{MockRuntime, SYSTEM_ACTOR_CODE_ID};
use fil_actors_runtime::SYSTEM_ACTOR_ADDR;
use fvm_ipld_encoding::ipld_block::IpldBlock;
use fvm_shared::MethodNum;

fn new_rt() -> MockRuntime { MockRuntime { receiver: fvm_shared::address::Address::new_id(8001), ..Default::default() } }

fn construct(rt: &MockRuntime) {
    rt.expect_validate_caller_any();
    rt.set_caller(*SYSTEM_ACTOR_CODE_ID, SYSTEM_ACTOR_ADDR);
    rt.call::<DelegatorActor>(Method::Constructor as MethodNum, None).unwrap();
    rt.verify();
}

#[test]
fn hamt_roots_update_and_state_persists() {
    use k256::ecdsa::{signature::hazmat::PrehashSigner, RecoveryId, Signature as EcdsaSignature, SigningKey, VerifyingKey};
    use rlp::RlpStream;

    let rt = new_rt();
    construct(&rt);

    let st0: State = Runtime::state(&rt).unwrap();
    let init_mappings = st0.mappings;
    let init_nonces = st0.nonces;

    // Apply a single delegation using chain_id 0 and nonce 0.
    let sk = SigningKey::from_bytes(&[0x42u8; 32].into()).unwrap();
    let vk = VerifyingKey::from(&sk);
    let delegate = EthAddress::from_id(1234);

    let mut s = RlpStream::new_list(3);
    s.append(&0u64);
    s.append(&delegate.as_ref());
    s.append(&0u64);
    let mut pre = vec![0x05u8];
    pre.extend_from_slice(&s.out());
    let mut d = [0u8; 32];
    d.copy_from_slice(&rt.hash(fvm_shared::crypto::hash::SupportedHashes::Keccak256, &pre));
    let sig: EcdsaSignature = sk.sign_prehash(&d).unwrap();
    let recid = RecoveryId::trial_recovery_from_prehash(&vk, &d, &sig).unwrap();

    rt.expect_validate_caller_any();
    let res = rt.call::<DelegatorActor>(
        Method::ApplyDelegations as MethodNum,
        IpldBlock::serialize_dag_cbor(&ApplyDelegationsParams { list: vec![DelegationParam {
            chain_id: 0, address: delegate, nonce: 0, y_parity: recid.to_byte(), r: sig.r().to_bytes().into(), s: sig.s().to_bytes().into()
        }]}).unwrap(),
    );
    assert!(res.unwrap().is_none());

    // State roots updated.
    let st1: State = Runtime::state(&rt).unwrap();
    assert_ne!(st1.mappings, init_mappings, "mapping HAMT root should change");
    assert_ne!(st1.nonces, init_nonces, "nonces HAMT root should change");

    // Sanity: a subsequent Apply with the same tuple fails (nonce bump persisted).
    rt.expect_validate_caller_any();
    let err = rt
        .call::<DelegatorActor>(
            Method::ApplyDelegations as MethodNum,
            IpldBlock::serialize_dag_cbor(&ApplyDelegationsParams { list: vec![DelegationParam {
                chain_id: 0, address: delegate, nonce: 0, y_parity: recid.to_byte(), r: sig.r().to_bytes().into(), s: sig.s().to_bytes().into()
            }]}).unwrap(),
        )
        .unwrap_err();
    assert_eq!(err.exit_code().value(), fvm_shared::error::ExitCode::USR_ILLEGAL_ARGUMENT.value());
}
