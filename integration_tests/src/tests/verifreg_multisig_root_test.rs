use export_macro::vm_test;
use fil_actor_init::ExecReturn;
use fil_actor_multisig::{Method as MsigMethod, ProposeParams, TxnIDParams};
use fil_actor_verifreg::{
    AddVerifierParams, DataCap, Method as VerifrregMethod, State as VerifrregState,
};
use fil_actors_runtime::cbor::serialize;
use fil_actors_runtime::runtime::Policy;
use fil_actors_runtime::test_utils::*;
use fil_actors_runtime::{INIT_ACTOR_ADDR, VERIFIED_REGISTRY_ACTOR_ADDR};
use fvm_shared::address::Address;
use fvm_shared::bigint::Zero;
use fvm_shared::econ::TokenAmount;
use vm_api::VM;
use vm_api::util::{DynBlockstore, apply_ok, get_state, mutate_state};

use crate::util::{assert_invariants, create_accounts};

/// Creates a multisig actor with the given signers and threshold
fn create_msig(v: &dyn VM, signers: &[Address], threshold: u64) -> Address {
    assert!(!signers.is_empty());
    let msig_ctor_params = serialize(
        &fil_actor_multisig::ConstructorParams {
            signers: signers.into(),
            num_approvals_threshold: threshold,
            unlock_duration: 0,
            start_epoch: 0,
        },
        "multisig ctor params",
    )
    .unwrap();
    let msig_ctor_ret: ExecReturn = apply_ok(
        v,
        &signers[0],
        &INIT_ACTOR_ADDR,
        &TokenAmount::from_atto(0),
        fil_actor_init::Method::Exec as u64,
        Some(fil_actor_init::ExecParams {
            code_cid: *MULTISIG_ACTOR_CODE_ID,
            constructor_params: msig_ctor_params,
        }),
    )
    .deserialize()
    .unwrap();
    msig_ctor_ret.id_address
}

#[vm_test]
pub fn test_multisig_as_verifreg_root_addverifier(v: &dyn VM) {
    // Create regular accounts for signers
    let addrs = create_accounts(v, 4, &TokenAmount::from_whole(10_000));
    let alice = addrs[0];
    let bob = addrs[1];
    let _charlie = addrs[2];
    let verifier_to_add = addrs[3];

    // Create a multisig with Alice and Bob as signers, threshold 2
    // This multisig will be the f080 root key holder
    let root_multisig_addr = create_msig(v, &[alice, bob], 2);

    // Update the existing verifreg actor's root key to our multisig
    mutate_state::<VerifrregState, _>(v, &VERIFIED_REGISTRY_ACTOR_ADDR, |st| {
        st.root_key = root_multisig_addr;
    });

    // Verify the verifreg state has the correct root key
    let updated_verifreg_st: VerifrregState = get_state(v, &VERIFIED_REGISTRY_ACTOR_ADDR).unwrap();
    assert_eq!(root_multisig_addr, updated_verifreg_st.root_key);

    // Define the verifier allowance (must be >= minimum_verified_allocation_size)
    let verifier_allowance = DataCap::from(1048576_u64); // 1 MiB

    // Prepare the addverifier call parameters
    let add_verifier_params =
        AddVerifierParams { address: verifier_to_add, allowance: verifier_allowance.clone() };

    // Step 1: Alice proposes in the multisig to call addverifier on verifreg
    let propose_addverifier_params = ProposeParams {
        to: VERIFIED_REGISTRY_ACTOR_ADDR,
        value: TokenAmount::zero(),
        method: VerifrregMethod::AddVerifier as u64,
        params: serialize(&add_verifier_params, "add verifier params").unwrap(),
    };

    apply_ok(
        v,
        &alice,
        &root_multisig_addr,
        &TokenAmount::zero(),
        MsigMethod::Propose as u64,
        Some(propose_addverifier_params),
    );

    // Step 2: Bob approves the proposal (this should execute the addverifier call)
    let approve_params = TxnIDParams {
        id: fil_actor_multisig::TxnID(0),
        proposal_hash: vec![], // hash optional
    };

    apply_ok(
        v,
        &bob,
        &root_multisig_addr,
        &TokenAmount::zero(),
        MsigMethod::Approve as u64,
        Some(approve_params),
    );

    // Verify that the verifier was successfully added to the verifreg actor
    // We can verify this by checking the verifreg state for the verifier
    let final_verifreg_st: VerifrregState = get_state(v, &VERIFIED_REGISTRY_ACTOR_ADDR).unwrap();
    let store = DynBlockstore::wrap(v.blockstore());
    let verifier_cap = final_verifreg_st.get_verifier_cap(&store, &verifier_to_add).unwrap();
    assert!(verifier_cap.is_some(), "Verifier should have been added");
    assert_eq!(verifier_allowance, verifier_cap.unwrap(), "Verifier allowance should match");

    assert_invariants(v, &Policy::default(), None);
}

#[vm_test]
pub fn test_multisig_as_verifreg_root_addverifier_fails_without_threshold(v: &dyn VM) {
    // Create regular accounts for signers
    let addrs = create_accounts(v, 4, &TokenAmount::from_whole(10_000));
    let alice = addrs[0];
    let bob = addrs[1];
    let charlie = addrs[2];
    let verifier_to_add = addrs[3];

    // Create a multisig with Alice, Bob, and Charlie as signers, threshold 3
    // This requires all three to approve
    let root_multisig_addr = create_msig(v, &[alice, bob, charlie], 3);

    // Update the existing verifreg actor's root key to our multisig
    mutate_state::<VerifrregState, _>(v, &VERIFIED_REGISTRY_ACTOR_ADDR, |st| {
        st.root_key = root_multisig_addr;
    });

    // Define the verifier allowance
    let verifier_allowance = DataCap::from(1048576_u64); // 1 MiB

    // Prepare the addverifier call parameters
    let add_verifier_params =
        AddVerifierParams { address: verifier_to_add, allowance: verifier_allowance.clone() };

    // Step 1: Alice proposes in the multisig to call addverifier on verifreg
    let propose_addverifier_params = ProposeParams {
        to: VERIFIED_REGISTRY_ACTOR_ADDR,
        value: TokenAmount::zero(),
        method: VerifrregMethod::AddVerifier as u64,
        params: serialize(&add_verifier_params, "add verifier params").unwrap(),
    };

    apply_ok(
        v,
        &alice,
        &root_multisig_addr,
        &TokenAmount::zero(),
        MsigMethod::Propose as u64,
        Some(propose_addverifier_params),
    );

    // Step 2: Bob approves the proposal (but threshold is 3, so this won't execute)
    let approve_params = TxnIDParams {
        id: fil_actor_multisig::TxnID(0),
        proposal_hash: vec![], // hash optional
    };

    apply_ok(
        v,
        &bob,
        &root_multisig_addr,
        &TokenAmount::zero(),
        MsigMethod::Approve as u64,
        Some(approve_params.clone()),
    );

    // Verify that the verifier was NOT added because threshold wasn't met
    let verifreg_st: VerifrregState = get_state(v, &VERIFIED_REGISTRY_ACTOR_ADDR).unwrap();
    let store = DynBlockstore::wrap(v.blockstore());
    let verifier_cap = verifreg_st.get_verifier_cap(&store, &verifier_to_add).unwrap();
    assert!(
        verifier_cap.is_none(),
        "Verifier should NOT have been added without meeting threshold"
    );

    // Step 3: Charlie approves the proposal (now threshold is met, should execute)
    apply_ok(
        v,
        &charlie,
        &root_multisig_addr,
        &TokenAmount::zero(),
        MsigMethod::Approve as u64,
        Some(approve_params),
    );

    // Now verify that the verifier was successfully added
    let final_verifreg_st: VerifrregState = get_state(v, &VERIFIED_REGISTRY_ACTOR_ADDR).unwrap();
    let final_verifier_cap = final_verifreg_st.get_verifier_cap(&store, &verifier_to_add).unwrap();
    assert!(
        final_verifier_cap.is_some(),
        "Verifier should have been added after meeting threshold"
    );
    assert_eq!(verifier_allowance, final_verifier_cap.unwrap(), "Verifier allowance should match");

    assert_invariants(v, &Policy::default(), None);
}
