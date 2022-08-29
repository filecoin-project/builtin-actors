use fil_actor_verifreg::{
    AddVerifierClientParams, RemoveDataCapParams, RemoveDataCapRequest, RemoveDataCapReturn,
    SIGNATURE_DOMAIN_SEPARATION_REMOVE_DATA_CAP,
};
use fil_actor_verifreg::{AddrPairKey, Method as VerifregMethod};
use fil_actor_verifreg::{RemoveDataCapProposal, RemoveDataCapProposalID, State as VerifregState};
use fil_actors_runtime::cbor::serialize;
use fil_actors_runtime::{make_map_with_root_and_bitwidth, VERIFIED_REGISTRY_ACTOR_ADDR};
use fvm_ipld_blockstore::MemoryBlockstore;
use fvm_ipld_encoding::to_vec;
use fvm_shared::bigint::bigint_ser::BigIntDe;
use fvm_shared::bigint::{BigInt, Zero};
use fvm_shared::crypto::signature::{Signature, SignatureType};
use fvm_shared::econ::TokenAmount;
use fvm_shared::sector::StoragePower;
use fvm_shared::HAMT_BIT_WIDTH;
use std::ops::{Div, Sub};
use test_vm::util::{add_verifier, apply_ok, create_accounts};
use test_vm::{ExpectInvocation, TEST_VERIFREG_ROOT_ADDR, VM};

#[test]
fn remove_datacap_simple_successful_path() {
    let store = MemoryBlockstore::new();
    let v = VM::new_with_singletons(&store);
    let addrs = create_accounts(&v, 4, TokenAmount::from(10_000e18 as i128));
    let (verifier1, verifier2, verified_client) = (addrs[0], addrs[1], addrs[2]);

    let verifier1_id_addr = v.normalize_address(&verifier1).unwrap();
    let verifier2_id_addr = v.normalize_address(&verifier2).unwrap();
    let verified_client_id_addr = v.normalize_address(&verified_client).unwrap();
    let verifier_allowance = StoragePower::from(2 * 1048576);
    let allowance_to_remove: StoragePower = verifier_allowance.clone().div(2);

    // register verifier1 and verifier2
    add_verifier(&v, verifier1, verifier_allowance.clone());
    add_verifier(&v, verifier2, verifier_allowance.clone());

    // register the verified client
    let add_verified_client_params =
        AddVerifierClientParams { address: verified_client, allowance: verifier_allowance.clone() };
    let add_verified_client_params_ser =
        serialize(&add_verified_client_params, "add verifier params").unwrap();
    apply_ok(
        &v,
        verifier1,
        *VERIFIED_REGISTRY_ACTOR_ADDR,
        TokenAmount::zero(),
        VerifregMethod::AddVerifiedClient as u64,
        add_verified_client_params,
    );

    ExpectInvocation {
        to: *VERIFIED_REGISTRY_ACTOR_ADDR,
        method: VerifregMethod::AddVerifiedClient as u64,
        params: Some(add_verified_client_params_ser),
        subinvocs: Some(vec![]),
        ..Default::default()
    }
    .matches(v.take_invocations().last().unwrap());

    // state checks on the 2 verifiers and the client
    let mut v_st = v.get_state::<VerifregState>(*VERIFIED_REGISTRY_ACTOR_ADDR).unwrap();
    let verifiers =
        make_map_with_root_and_bitwidth::<_, BigIntDe>(&v_st.verifiers, &store, HAMT_BIT_WIDTH)
            .unwrap();

    let BigIntDe(verifier1_data_cap) =
        verifiers.get(&verifier1_id_addr.to_bytes()).unwrap().unwrap();
    assert_eq!(BigInt::zero(), *verifier1_data_cap);

    let BigIntDe(verifier2_data_cap) =
        verifiers.get(&verifier2_id_addr.to_bytes()).unwrap().unwrap();
    assert_eq!(verifier_allowance, *verifier2_data_cap);

    let mut verified_clients = make_map_with_root_and_bitwidth::<_, BigIntDe>(
        &v_st.verified_clients,
        &store,
        HAMT_BIT_WIDTH,
    )
    .unwrap();

    let BigIntDe(data_cap) =
        verified_clients.get(&verified_client_id_addr.to_bytes()).unwrap().unwrap();
    assert_eq!(*data_cap, verifier_allowance);

    let mut proposal_ids = make_map_with_root_and_bitwidth::<_, RemoveDataCapProposalID>(
        &v_st.remove_data_cap_proposal_ids,
        &store,
        HAMT_BIT_WIDTH,
    )
    .unwrap();

    assert!(proposal_ids
        .get(&AddrPairKey::new(verifier1_id_addr, verified_client_id_addr).to_bytes())
        .unwrap()
        .is_none());

    assert!(proposal_ids
        .get(&AddrPairKey::new(verifier2_id_addr, verified_client_id_addr).to_bytes())
        .unwrap()
        .is_none());

    // remove half the client's allowance
    let mut verifier1_proposal = RemoveDataCapProposal {
        verified_client: verified_client_id_addr,
        data_cap_amount: allowance_to_remove.clone(),
        removal_proposal_id: RemoveDataCapProposalID { id: 0 },
    };

    let mut verifier1_proposal_ser = to_vec(&verifier1_proposal).unwrap();
    let mut verifier1_payload = SIGNATURE_DOMAIN_SEPARATION_REMOVE_DATA_CAP.to_vec();
    verifier1_payload.append(&mut verifier1_proposal_ser);

    let mut verifier2_proposal = RemoveDataCapProposal {
        verified_client: verified_client_id_addr,
        data_cap_amount: allowance_to_remove.clone(),
        removal_proposal_id: RemoveDataCapProposalID { id: 0 },
    };

    let mut verifier2_proposal_ser = to_vec(&verifier2_proposal).unwrap();
    let mut verifier2_payload = SIGNATURE_DOMAIN_SEPARATION_REMOVE_DATA_CAP.to_vec();
    verifier2_payload.append(&mut verifier2_proposal_ser);

    let mut remove_datacap_params = RemoveDataCapParams {
        verified_client_to_remove: verified_client_id_addr,
        data_cap_amount_to_remove: allowance_to_remove.clone(),
        verifier_request_1: RemoveDataCapRequest {
            verifier: verifier1_id_addr,
            signature: Signature { sig_type: SignatureType::Secp256k1, bytes: verifier1_payload },
        },
        verifier_request_2: RemoveDataCapRequest {
            verifier: verifier2_id_addr,
            signature: Signature { sig_type: SignatureType::Secp256k1, bytes: verifier2_payload },
        },
    };

    let mut remove_datacap_params_ser =
        serialize(&remove_datacap_params, "add verifier params").unwrap();

    let remove_datacap_ret: RemoveDataCapReturn = apply_ok(
        &v,
        TEST_VERIFREG_ROOT_ADDR,
        *VERIFIED_REGISTRY_ACTOR_ADDR,
        TokenAmount::zero(),
        VerifregMethod::RemoveVerifiedClientDataCap as u64,
        remove_datacap_params,
    )
    .deserialize()
    .unwrap();

    ExpectInvocation {
        to: *VERIFIED_REGISTRY_ACTOR_ADDR,
        method: VerifregMethod::RemoveVerifiedClientDataCap as u64,
        params: Some(remove_datacap_params_ser),
        subinvocs: Some(vec![]),
        ..Default::default()
    }
    .matches(v.take_invocations().last().unwrap());

    assert_eq!(verified_client_id_addr, remove_datacap_ret.verified_client);
    assert_eq!(allowance_to_remove, remove_datacap_ret.data_cap_removed);

    v_st = v.get_state::<VerifregState>(*VERIFIED_REGISTRY_ACTOR_ADDR).unwrap();

    // confirm client's allowance has fallen by half
    verified_clients = make_map_with_root_and_bitwidth::<_, BigIntDe>(
        &v_st.verified_clients,
        &store,
        HAMT_BIT_WIDTH,
    )
    .unwrap();

    let BigIntDe(data_cap) =
        verified_clients.get(&verified_client_id_addr.to_bytes()).unwrap().unwrap();

    assert_eq!(*data_cap, verifier_allowance.sub(allowance_to_remove.clone()));

    // confirm proposalIds has changed as expected
    proposal_ids =
        make_map_with_root_and_bitwidth(&v_st.remove_data_cap_proposal_ids, &store, HAMT_BIT_WIDTH)
            .unwrap();

    let verifier1_proposal_id: &RemoveDataCapProposalID = proposal_ids
        .get(&AddrPairKey::new(verifier1_id_addr, verified_client_id_addr).to_bytes())
        .unwrap()
        .unwrap();

    assert_eq!(1u64, verifier1_proposal_id.id);

    let verifier2_proposal_id: &RemoveDataCapProposalID = proposal_ids
        .get(&AddrPairKey::new(verifier2_id_addr, verified_client_id_addr).to_bytes())
        .unwrap()
        .unwrap();

    assert_eq!(1u64, verifier2_proposal_id.id);

    // remove the second half of the client's allowance, this causes the client to be deleted

    verifier1_proposal = RemoveDataCapProposal {
        verified_client: verified_client_id_addr,
        data_cap_amount: allowance_to_remove.clone(),
        removal_proposal_id: verifier1_proposal_id.clone(),
    };

    verifier1_proposal_ser = to_vec(&verifier1_proposal).unwrap();
    verifier1_payload = SIGNATURE_DOMAIN_SEPARATION_REMOVE_DATA_CAP.to_vec();
    verifier1_payload.append(&mut verifier1_proposal_ser);

    verifier2_proposal = RemoveDataCapProposal {
        verified_client: verified_client_id_addr,
        data_cap_amount: allowance_to_remove.clone(),
        removal_proposal_id: verifier2_proposal_id.clone(),
    };

    verifier2_proposal_ser = to_vec(&verifier2_proposal).unwrap();
    verifier2_payload = SIGNATURE_DOMAIN_SEPARATION_REMOVE_DATA_CAP.to_vec();
    verifier2_payload.append(&mut verifier2_proposal_ser);

    remove_datacap_params = RemoveDataCapParams {
        verified_client_to_remove: verified_client_id_addr,
        data_cap_amount_to_remove: allowance_to_remove.clone(),
        verifier_request_1: RemoveDataCapRequest {
            verifier: verifier1_id_addr,
            signature: Signature { sig_type: SignatureType::Secp256k1, bytes: verifier1_payload },
        },
        verifier_request_2: RemoveDataCapRequest {
            verifier: verifier2_id_addr,
            signature: Signature { sig_type: SignatureType::Secp256k1, bytes: verifier2_payload },
        },
    };

    remove_datacap_params_ser = serialize(&remove_datacap_params, "add verifier params").unwrap();

    let remove_datacap_ret: RemoveDataCapReturn = apply_ok(
        &v,
        TEST_VERIFREG_ROOT_ADDR,
        *VERIFIED_REGISTRY_ACTOR_ADDR,
        TokenAmount::zero(),
        VerifregMethod::RemoveVerifiedClientDataCap as u64,
        remove_datacap_params,
    )
    .deserialize()
    .unwrap();

    ExpectInvocation {
        to: *VERIFIED_REGISTRY_ACTOR_ADDR,
        method: VerifregMethod::RemoveVerifiedClientDataCap as u64,
        params: Some(remove_datacap_params_ser),
        subinvocs: Some(vec![]),
        ..Default::default()
    }
    .matches(v.take_invocations().last().unwrap());

    assert_eq!(verified_client_id_addr, remove_datacap_ret.verified_client);
    assert_eq!(allowance_to_remove, remove_datacap_ret.data_cap_removed);

    // confirm client has been removed entirely

    v_st = v.get_state::<VerifregState>(*VERIFIED_REGISTRY_ACTOR_ADDR).unwrap();
    verified_clients = make_map_with_root_and_bitwidth::<_, BigIntDe>(
        &v_st.verified_clients,
        &store,
        HAMT_BIT_WIDTH,
    )
    .unwrap();

    assert!(verified_clients.get(&verified_client_id_addr.to_bytes()).unwrap().is_none());

    // confirm proposalIds has changed as expected
    proposal_ids =
        make_map_with_root_and_bitwidth(&v_st.remove_data_cap_proposal_ids, &store, HAMT_BIT_WIDTH)
            .unwrap();

    let verifier1_proposal_id: &RemoveDataCapProposalID = proposal_ids
        .get(&AddrPairKey::new(verifier1_id_addr, verified_client_id_addr).to_bytes())
        .unwrap()
        .unwrap();

    assert_eq!(2u64, verifier1_proposal_id.id);

    let verifier2_proposal_id: &RemoveDataCapProposalID = proposal_ids
        .get(&AddrPairKey::new(verifier2_id_addr, verified_client_id_addr).to_bytes())
        .unwrap()
        .unwrap();

    assert_eq!(2u64, verifier2_proposal_id.id);
    v.assert_state_invariants();
}
