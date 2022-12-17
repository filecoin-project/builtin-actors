use std::ops::{Div, Sub};

use fil_actor_account::types::AuthenticateMessageParams;
use fil_actor_account::Method as AccountMethod;
use fvm_ipld_blockstore::MemoryBlockstore;
use fvm_ipld_encoding::{to_vec, RawBytes};
use fvm_shared::bigint::bigint_ser::BigIntDe;
use fvm_shared::bigint::{BigInt, Zero};
use fvm_shared::crypto::signature::{Signature, SignatureType};
use fvm_shared::econ::TokenAmount;
use fvm_shared::error::ExitCode;
use fvm_shared::sector::StoragePower;
use fvm_shared::HAMT_BIT_WIDTH;
use num_traits::ToPrimitive;

use fil_actor_datacap::{
    DestroyParams, Method as DataCapMethod, MintParams, State as DataCapState,
};
use fil_actor_verifreg::{
    AddVerifiedClientParams, DataCap, RemoveDataCapParams, RemoveDataCapRequest,
    RemoveDataCapReturn, SIGNATURE_DOMAIN_SEPARATION_REMOVE_DATA_CAP,
};
use fil_actor_verifreg::{AddrPairKey, Method as VerifregMethod};
use fil_actor_verifreg::{RemoveDataCapProposal, RemoveDataCapProposalID, State as VerifregState};
use fil_actors_runtime::{
    make_map_with_root_and_bitwidth, DATACAP_TOKEN_ACTOR_ADDR, STORAGE_MARKET_ACTOR_ADDR,
    VERIFIED_REGISTRY_ACTOR_ADDR,
};
use fvm_ipld_encoding::ipld_block::IpldBlock;
use test_vm::util::{apply_code, apply_ok, create_accounts, verifreg_add_verifier};
use test_vm::{ExpectInvocation, TEST_VERIFREG_ROOT_ADDR, VM};

#[test]
fn remove_datacap_simple_successful_path() {
    let store = MemoryBlockstore::new();
    let v = VM::new_with_singletons(&store);
    let addrs = create_accounts(&v, 4, TokenAmount::from_whole(10_000));
    let (verifier1, verifier2, verified_client) = (addrs[0], addrs[1], addrs[2]);

    let verifier1_id_addr = v.normalize_address(&verifier1).unwrap();
    let verifier2_id_addr = v.normalize_address(&verifier2).unwrap();
    let verified_client_id_addr = v.normalize_address(&verified_client).unwrap();
    let verifier_allowance = StoragePower::from(2 * 1048576u64);
    let allowance_to_remove: StoragePower = verifier_allowance.clone().div(2);

    // register verifier1 and verifier2
    verifreg_add_verifier(&v, verifier1, verifier_allowance.clone());
    verifreg_add_verifier(&v, verifier2, verifier_allowance.clone());

    // register the verified client
    let add_verified_client_params =
        AddVerifiedClientParams { address: verified_client, allowance: verifier_allowance.clone() };
    let mint_params = MintParams {
        to: verified_client,
        amount: TokenAmount::from_whole(verifier_allowance.to_i64().unwrap()),
        operators: vec![STORAGE_MARKET_ACTOR_ADDR],
    };
    apply_ok(
        &v,
        verifier1,
        VERIFIED_REGISTRY_ACTOR_ADDR,
        TokenAmount::zero(),
        VerifregMethod::AddVerifiedClient as u64,
        Some(add_verified_client_params.clone()),
    );

    ExpectInvocation {
        to: VERIFIED_REGISTRY_ACTOR_ADDR,
        method: VerifregMethod::AddVerifiedClient as u64,
        params: Some(IpldBlock::serialize_cbor(&add_verified_client_params).unwrap()),
        subinvocs: Some(vec![ExpectInvocation {
            to: DATACAP_TOKEN_ACTOR_ADDR,
            method: DataCapMethod::MintExported as u64,
            params: Some(IpldBlock::serialize_cbor(&mint_params).unwrap()),
            subinvocs: None,
            ..Default::default()
        }]),
        ..Default::default()
    }
    .matches(v.take_invocations().last().unwrap());

    // state checks on the 2 verifiers and the client
    let mut v_st = v.get_state::<VerifregState>(VERIFIED_REGISTRY_ACTOR_ADDR).unwrap();
    let verifiers =
        make_map_with_root_and_bitwidth::<_, BigIntDe>(&v_st.verifiers, &store, HAMT_BIT_WIDTH)
            .unwrap();

    let BigIntDe(verifier1_data_cap) =
        verifiers.get(&verifier1_id_addr.to_bytes()).unwrap().unwrap();
    assert_eq!(BigInt::zero(), *verifier1_data_cap);

    let BigIntDe(verifier2_data_cap) =
        verifiers.get(&verifier2_id_addr.to_bytes()).unwrap().unwrap();
    assert_eq!(verifier_allowance, *verifier2_data_cap);

    let token_st = v.get_state::<DataCapState>(DATACAP_TOKEN_ACTOR_ADDR).unwrap();
    let balance = token_st.balance(&store, verified_client_id_addr.id().unwrap()).unwrap();
    assert_eq!(balance, TokenAmount::from_whole(verifier_allowance.to_i64().unwrap()));

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

    let remove_datacap_ret: RemoveDataCapReturn = apply_ok(
        &v,
        TEST_VERIFREG_ROOT_ADDR,
        VERIFIED_REGISTRY_ACTOR_ADDR,
        TokenAmount::zero(),
        VerifregMethod::RemoveVerifiedClientDataCap as u64,
        Some(remove_datacap_params.clone()),
    )
    .deserialize()
    .unwrap();

    expect_remove_datacap(
        &remove_datacap_params,
        RemoveDataCapProposalID { id: 0 },
        RemoveDataCapProposalID { id: 0 },
    )
    .matches(v.take_invocations().last().unwrap());

    assert_eq!(verified_client_id_addr, remove_datacap_ret.verified_client);
    assert_eq!(allowance_to_remove, remove_datacap_ret.data_cap_removed);

    // confirm client's allowance has fallen by half
    let token_st = v.get_state::<DataCapState>(DATACAP_TOKEN_ACTOR_ADDR).unwrap();
    let balance = token_st.balance(&store, verified_client_id_addr.id().unwrap()).unwrap();
    assert_eq!(
        balance,
        TokenAmount::from_whole(verifier_allowance.sub(&allowance_to_remove).to_i64().unwrap())
    );

    v_st = v.get_state::<VerifregState>(VERIFIED_REGISTRY_ACTOR_ADDR).unwrap();
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

    let remove_datacap_ret: RemoveDataCapReturn = apply_ok(
        &v,
        TEST_VERIFREG_ROOT_ADDR,
        VERIFIED_REGISTRY_ACTOR_ADDR,
        TokenAmount::zero(),
        VerifregMethod::RemoveVerifiedClientDataCap as u64,
        Some(remove_datacap_params.clone()),
    )
    .deserialize()
    .unwrap();

    expect_remove_datacap(
        &remove_datacap_params,
        RemoveDataCapProposalID { id: 1 },
        RemoveDataCapProposalID { id: 1 },
    )
    .matches(v.take_invocations().last().unwrap());

    assert_eq!(verified_client_id_addr, remove_datacap_ret.verified_client);
    assert_eq!(allowance_to_remove, remove_datacap_ret.data_cap_removed);

    // confirm client has no balance
    let token_st = v.get_state::<DataCapState>(DATACAP_TOKEN_ACTOR_ADDR).unwrap();
    let balance = token_st.balance(&store, verified_client_id_addr.id().unwrap()).unwrap();
    assert_eq!(balance, TokenAmount::zero());

    // confirm proposalIds has changed as expected
    v_st = v.get_state::<VerifregState>(VERIFIED_REGISTRY_ACTOR_ADDR).unwrap();
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

#[test]
fn remove_datacap_fails_on_verifreg() {
    let store = MemoryBlockstore::new();
    let v = VM::new_with_singletons(&store);
    let addrs = create_accounts(&v, 2, TokenAmount::from_whole(10_000));
    let (verifier1, verifier2) = (addrs[0], addrs[1]);

    let verifier1_id_addr = v.normalize_address(&verifier1).unwrap();
    let verifier2_id_addr = v.normalize_address(&verifier2).unwrap();
    let verifier_allowance = StoragePower::from(2 * 1048576u64);
    let allowance_to_remove: StoragePower = DataCap::from(100);

    // register verifier1 and verifier2
    verifreg_add_verifier(&v, verifier1, verifier_allowance.clone());
    verifreg_add_verifier(&v, verifier2, verifier_allowance);

    let remove_proposal = RemoveDataCapProposal {
        verified_client: VERIFIED_REGISTRY_ACTOR_ADDR,
        data_cap_amount: allowance_to_remove.clone(),
        removal_proposal_id: RemoveDataCapProposalID { id: 0 },
    };

    let mut remove_proposal_ser = to_vec(&remove_proposal).unwrap();
    let mut remove_proposal_payload = SIGNATURE_DOMAIN_SEPARATION_REMOVE_DATA_CAP.to_vec();
    remove_proposal_payload.append(&mut remove_proposal_ser);

    let remove_datacap_params = RemoveDataCapParams {
        verified_client_to_remove: VERIFIED_REGISTRY_ACTOR_ADDR,
        data_cap_amount_to_remove: allowance_to_remove,
        verifier_request_1: RemoveDataCapRequest {
            verifier: verifier1_id_addr,
            signature: Signature {
                sig_type: SignatureType::Secp256k1,
                bytes: remove_proposal_payload.clone(),
            },
        },
        verifier_request_2: RemoveDataCapRequest {
            verifier: verifier2_id_addr,
            signature: Signature {
                sig_type: SignatureType::Secp256k1,
                bytes: remove_proposal_payload,
            },
        },
    };

    apply_code(
        &v,
        TEST_VERIFREG_ROOT_ADDR,
        VERIFIED_REGISTRY_ACTOR_ADDR,
        TokenAmount::zero(),
        VerifregMethod::RemoveVerifiedClientDataCap as u64,
        Some(remove_datacap_params),
        ExitCode::USR_ILLEGAL_ARGUMENT,
    );

    v.assert_state_invariants();
}

fn expect_remove_datacap(
    params: &RemoveDataCapParams,
    proposal_id1: RemoveDataCapProposalID,
    proposal_id2: RemoveDataCapProposalID,
) -> ExpectInvocation {
    let payload1 = [
        SIGNATURE_DOMAIN_SEPARATION_REMOVE_DATA_CAP,
        RawBytes::serialize(&RemoveDataCapProposal {
            removal_proposal_id: proposal_id1,
            data_cap_amount: params.data_cap_amount_to_remove.clone(),
            verified_client: params.verified_client_to_remove,
        })
        .unwrap()
        .bytes(),
    ]
    .concat();
    let payload2 = [
        SIGNATURE_DOMAIN_SEPARATION_REMOVE_DATA_CAP,
        RawBytes::serialize(&RemoveDataCapProposal {
            removal_proposal_id: proposal_id2,
            data_cap_amount: params.data_cap_amount_to_remove.clone(),
            verified_client: params.verified_client_to_remove,
        })
        .unwrap()
        .bytes(),
    ]
    .concat();
    ExpectInvocation {
        to: VERIFIED_REGISTRY_ACTOR_ADDR,
        method: VerifregMethod::RemoveVerifiedClientDataCap as u64,
        params: Some(IpldBlock::serialize_cbor(&params).unwrap()),
        code: Some(ExitCode::OK),
        subinvocs: Some(vec![
            ExpectInvocation {
                to: params.verifier_request_1.verifier,
                method: AccountMethod::AuthenticateMessageExported as u64,
                params: Some(
                    IpldBlock::serialize_cbor(&AuthenticateMessageParams {
                        signature: payload1.clone(),
                        message: payload1,
                    })
                    .unwrap(),
                ),
                code: Some(ExitCode::OK),
                subinvocs: None,
                ..Default::default()
            },
            ExpectInvocation {
                to: params.verifier_request_2.verifier,
                method: AccountMethod::AuthenticateMessageExported as u64,
                params: Some(
                    IpldBlock::serialize_cbor(&AuthenticateMessageParams {
                        signature: payload2.clone(),
                        message: payload2,
                    })
                    .unwrap(),
                ),
                code: Some(ExitCode::OK),
                subinvocs: None,
                ..Default::default()
            },
            ExpectInvocation {
                to: DATACAP_TOKEN_ACTOR_ADDR,
                method: DataCapMethod::BalanceExported as u64,
                params: Some(IpldBlock::serialize_cbor(&params.verified_client_to_remove).unwrap()),
                code: Some(ExitCode::OK),
                subinvocs: None,
                ..Default::default()
            },
            ExpectInvocation {
                to: DATACAP_TOKEN_ACTOR_ADDR,
                method: DataCapMethod::DestroyExported as u64,
                params: Some(
                    IpldBlock::serialize_cbor(&DestroyParams {
                        owner: params.verified_client_to_remove,
                        amount: TokenAmount::from_whole(
                            params.data_cap_amount_to_remove.to_i64().unwrap(),
                        ),
                    })
                    .unwrap(),
                ),
                code: Some(ExitCode::OK),
                subinvocs: None,
                ..Default::default()
            },
        ]),
        ..Default::default()
    }
}
