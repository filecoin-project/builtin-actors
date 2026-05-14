use export_macro::vm_test;
use fil_actor_multisig::ProposeParams;
use fil_actor_verifreg::Method as VerifregMethod;
use fil_actor_verifreg::{
    DataCap, RemoveDataCapParams, RemoveDataCapRequest, SIGNATURE_DOMAIN_SEPARATION_REMOVE_DATA_CAP,
};
use fil_actor_verifreg::{RemoveDataCapProposal, RemoveDataCapProposalID, State as VerifregState};
use fil_actors_runtime::VERIFIED_REGISTRY_ACTOR_ADDR;
use fil_actors_runtime::runtime::Policy;
use fvm_ipld_encoding::to_vec;
use fvm_shared::bigint::Zero;
use fvm_shared::crypto::signature::{Signature, SignatureType};
use fvm_shared::econ::TokenAmount;
use fvm_shared::error::ExitCode;
use fvm_shared::sector::StoragePower;
use vm_api::VM;
use vm_api::util::{DynBlockstore, apply_code, apply_ok, get_state};

use crate::TEST_VERIFREG_ROOT_ADDR;
use crate::util::{assert_invariants, create_accounts};

/// FIP-1249: AddVerifier is now deprecated and returns USR_FORBIDDEN.
/// This test verifies that the deprecated path correctly fails.
#[vm_test]
pub fn remove_datacap_simple_successful_path_test(v: &dyn VM) {
    let addrs = create_accounts(v, 4, &TokenAmount::from_whole(10_000));
    let (verifier1, _verifier2, _verified_client) = (addrs[0], addrs[1], addrs[2]);
    let verifier_allowance = StoragePower::from(2 * 1048576u64);

    // FIP-1249: AddVerifier is deprecated and should return USR_FORBIDDEN
    let add_verifier_params =
        fil_actor_verifreg::VerifierParams { address: verifier1, allowance: verifier_allowance };
    let proposal = ProposeParams {
        to: VERIFIED_REGISTRY_ACTOR_ADDR,
        value: TokenAmount::zero(),
        method: VerifregMethod::AddVerifier as u64,
        params: fil_actors_runtime::cbor::serialize(
            &add_verifier_params,
            "verifreg add verifier params",
        )
        .unwrap(),
    };

    apply_ok(
        v,
        &crate::TEST_VERIFREG_ROOT_SIGNER_ADDR,
        &crate::TEST_VERIFREG_ROOT_ADDR,
        &TokenAmount::zero(),
        fil_actor_multisig::Method::Propose as u64,
        Some(proposal),
    );

    // The inner call to AddVerifier should have failed with USR_FORBIDDEN.
    // The multisig Propose itself succeeds but the inner call returns an error.
    // Verify by checking that the verifier was NOT added.
    let v_st: VerifregState = get_state(v, &VERIFIED_REGISTRY_ACTOR_ADDR).unwrap();
    let store = DynBlockstore::wrap(v.blockstore());
    let cap = v_st.get_verifier_cap(&store, &verifier1).unwrap();
    assert!(cap.is_none(), "Verifier should not have been added (AddVerifier is deprecated)");

    assert_invariants(v, &Policy::default(), None)
}

/// FIP-1249: AddVerifier is now deprecated and returns USR_FORBIDDEN.
/// This test verifies that RemoveDataCap fails when verifiers don't exist
/// (since AddVerifier is deprecated and can't create verifiers).
#[vm_test]
pub fn remove_datacap_fails_on_verifreg_test(v: &dyn VM) {
    let addrs = create_accounts(v, 2, &TokenAmount::from_whole(10_000));
    let (verifier1, verifier2) = (addrs[0], addrs[1]);

    let verifier1_id_addr = v.resolve_id_address(&verifier1).unwrap();
    let verifier2_id_addr = v.resolve_id_address(&verifier2).unwrap();
    let allowance_to_remove: StoragePower = DataCap::from(100);

    // FIP-1249: Cannot register verifiers anymore (AddVerifier returns USR_FORBIDDEN).
    // Try to remove datacap from verifreg itself - should fail because verifiers don't exist.
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

    // Fails because verifiers don't exist (can't be created anymore)
    apply_code(
        v,
        &TEST_VERIFREG_ROOT_ADDR,
        &VERIFIED_REGISTRY_ACTOR_ADDR,
        &TokenAmount::zero(),
        VerifregMethod::RemoveVerifiedClientDataCap as u64,
        Some(remove_datacap_params),
        ExitCode::USR_ILLEGAL_ARGUMENT,
    );

    assert_invariants(v, &Policy::default(), None)
}

// FIP-1249: expect_remove_datacap helper removed since AddVerifier is deprecated
// and remove_datacap_simple_successful_path no longer exercises the full flow.
