use export_macro::vm_test;
use fil_actor_init::ExecReturn;
use fil_actor_multisig::{
    compute_proposal_hash, Method as MsigMethod, PendingTxnMap, ProposeParams, RemoveSignerParams,
    State as MsigState, SwapSignerParams, Transaction, TxnID, TxnIDParams, PENDING_TXN_CONFIG,
};
use fil_actors_runtime::cbor::serialize;
use fil_actors_runtime::runtime::Policy;
use fil_actors_runtime::test_utils::*;
use fil_actors_runtime::{INIT_ACTOR_ADDR, SYSTEM_ACTOR_ADDR};
use fvm_ipld_encoding::RawBytes;
use fvm_shared::address::Address;
use fvm_shared::bigint::Zero;
use fvm_shared::econ::TokenAmount;
use fvm_shared::error::ExitCode;
use fvm_shared::METHOD_SEND;
use std::collections::HashSet;
use std::iter::FromIterator;
use vm_api::trace::ExpectInvocation;
use vm_api::util::{apply_code, apply_ok, get_state, DynBlockstore};
use vm_api::VM;

use crate::expects::Expect;
use crate::util::{assert_invariants, create_accounts};

#[vm_test]
pub fn proposal_hash_test(v: &dyn VM) {
    let addrs = create_accounts(v, 3, &TokenAmount::from_whole(10_000));
    let sys_act_start_bal = v.actor(&SYSTEM_ACTOR_ADDR).unwrap().balance;
    let alice = addrs[0];
    let bob = addrs[1];
    let bob_id = bob.id().unwrap();
    let msig_addr = create_msig(v, &addrs, 2);
    let msig_id = msig_addr.id().unwrap();

    // fund msig and propose send funds to system actor
    let fil_delta = TokenAmount::from_nano(3);
    let propose_send_sys_params = ProposeParams {
        to: SYSTEM_ACTOR_ADDR,
        value: fil_delta.clone(),
        method: METHOD_SEND,
        params: RawBytes::default(),
    };
    apply_ok(
        v,
        &alice,
        &msig_addr,
        &fil_delta,
        MsigMethod::Propose as u64,
        Some(propose_send_sys_params),
    );

    let wrong_tx = Transaction {
        to: SYSTEM_ACTOR_ADDR,
        value: &fil_delta - TokenAmount::from_atto(1), // incorrect send amount not consistent with proposal
        method: METHOD_SEND,
        approved: vec![alice],
        params: RawBytes::default(),
    };

    let wrong_hash = compute_proposal_hash(&wrong_tx, v.primitives()).unwrap();

    let wrong_approval_params = TxnIDParams { id: TxnID(0), proposal_hash: wrong_hash.to_vec() };
    apply_code(
        v,
        &bob,
        &msig_addr,
        &TokenAmount::zero(),
        MsigMethod::Approve as u64,
        Some(wrong_approval_params),
        ExitCode::USR_ILLEGAL_ARGUMENT,
    );

    let correct_tx = Transaction {
        to: SYSTEM_ACTOR_ADDR,
        value: fil_delta.clone(),
        method: METHOD_SEND,
        approved: vec![alice],
        params: RawBytes::default(),
    };

    let correct_hash = compute_proposal_hash(&correct_tx, v.primitives()).unwrap();

    let correct_approval_params =
        TxnIDParams { id: TxnID(0), proposal_hash: correct_hash.to_vec() };
    apply_ok(
        v,
        &bob,
        &msig_addr,
        &TokenAmount::zero(),
        MsigMethod::Approve as u64,
        Some(correct_approval_params),
    );
    let expect = ExpectInvocation {
        from: bob_id,
        to: msig_addr,
        method: MsigMethod::Approve as u64,
        subinvocs: Some(vec![
            // Tx goes through to fund the system actor
            Expect::send(msig_id, SYSTEM_ACTOR_ADDR, Some(fil_delta.clone())),
        ]),
        ..Default::default()
    };
    expect.matches(v.take_invocations().last().unwrap());

    assert_eq!(sys_act_start_bal + fil_delta, v.actor(&SYSTEM_ACTOR_ADDR).unwrap().balance);
    assert_invariants(v, &Policy::default(), None)
}

pub fn test_delete_self_inner_test(v: &dyn VM, signers: u64, threshold: usize, remove_idx: usize) {
    let addrs = create_accounts(v, signers, &TokenAmount::from_whole(10_000));
    let msig_addr = create_msig(v, &addrs, threshold as u64);

    let remove_params =
        RemoveSignerParams { signer: addrs[remove_idx], decrease: threshold == addrs.len() };
    let remove_param_ser = serialize(&remove_params, "remove params").unwrap();
    let propose_remove_params = ProposeParams {
        to: msig_addr,
        value: TokenAmount::zero(),
        method: MsigMethod::RemoveSigner as u64,
        params: remove_param_ser,
    };

    // first proposal goes ok and should have txnid = 0
    apply_ok(
        v,
        &addrs[0],
        &msig_addr,
        &TokenAmount::zero(),
        MsigMethod::Propose as u64,
        Some(propose_remove_params),
    );

    // approval goes through
    let approve_remove_signer = TxnIDParams {
        id: TxnID(0),
        proposal_hash: vec![], // hash optional
    };
    for addr in addrs.iter().take(threshold).skip(1) {
        apply_ok(
            v,
            addr,
            &msig_addr,
            &TokenAmount::zero(),
            MsigMethod::Approve as u64,
            Some(approve_remove_signer.clone()),
        );
    }

    if addrs.len() > threshold {
        // txnid not found when third approval gets processed indicating that the transaction has gone through successfully
        apply_code(
            v,
            &addrs[threshold],
            &msig_addr,
            &TokenAmount::zero(),
            MsigMethod::Approve as u64,
            Some(approve_remove_signer),
            ExitCode::USR_NOT_FOUND,
        );
    }
    // assert tx queue empty
    check_txs(v, msig_addr, vec![]);
    // assert signers == original signers minus removed
    let old_signers: HashSet<Address> = HashSet::from_iter(addrs.clone());
    let st: MsigState = get_state(v, &msig_addr).unwrap();
    let new_signers: HashSet<Address> = HashSet::from_iter(st.signers);
    let diff: Vec<&Address> = old_signers.symmetric_difference(&new_signers).collect();
    assert_eq!(vec![&(addrs[remove_idx])], diff);
    assert_invariants(v, &Policy::default(), None)
}

#[vm_test]
pub fn swap_self_1_of_2_test(v: &dyn VM) {
    let addrs = create_accounts(v, 3, &TokenAmount::from_whole(10_000));
    let (alice, bob, chuck) = (addrs[0], addrs[1], addrs[2]);
    let msig_addr = create_msig(v, &[alice, bob], 1);
    let swap_params = SwapSignerParams { from: alice, to: chuck };
    let propose_swap_signer_params = ProposeParams {
        to: msig_addr,
        value: TokenAmount::zero(),
        method: MsigMethod::SwapSigner as u64,
        params: serialize(&swap_params, "swap params").unwrap(),
    };
    // alice succeeds when trying to execute the tx swapping alice for chuck
    apply_ok(
        v,
        &alice,
        &msig_addr,
        &TokenAmount::zero(),
        MsigMethod::Propose as u64,
        Some(propose_swap_signer_params),
    );
    let st: MsigState = get_state(v, &msig_addr).unwrap();
    assert_eq!(vec![bob, chuck], st.signers);
    assert_invariants(v, &Policy::default(), None);
}

#[vm_test]
pub fn swap_self_2_of_3_test(v: &dyn VM) {
    let addrs = create_accounts(v, 4, &TokenAmount::from_whole(10_000));
    let (alice, bob, chuck, dinesh) = (addrs[0], addrs[1], addrs[2], addrs[3]);

    let msig_addr = create_msig(v, &vec![alice, bob, chuck], 2);

    // Case 1: swapped out is proposer, swap alice for dinesh
    let swap_params = SwapSignerParams { from: alice, to: dinesh };
    let propose_swap_signer_params = ProposeParams {
        to: msig_addr,
        value: TokenAmount::zero(),
        method: MsigMethod::SwapSigner as u64,
        params: serialize(&swap_params, "swap params").unwrap(),
    };

    // proposal from swapped addr goes ok with txnid 0
    apply_ok(
        v,
        &alice,
        &msig_addr,
        &TokenAmount::zero(),
        MsigMethod::Propose as u64,
        Some(propose_swap_signer_params),
    );

    // approval goes through
    let approve_swap_signer_params = TxnIDParams { id: TxnID(0), proposal_hash: vec![] };
    apply_ok(
        v,
        &bob,
        &msig_addr,
        &TokenAmount::zero(),
        MsigMethod::Approve as u64,
        Some(approve_swap_signer_params),
    );
    let st: MsigState = get_state(v, &msig_addr).unwrap();
    assert_eq!(vec![bob, chuck, dinesh], st.signers);

    // Case 2: swapped out is approver, swap dinesh for alice, dinesh is removed
    let swap_params = SwapSignerParams { from: dinesh, to: alice };
    let propose_swap_signer_params = ProposeParams {
        to: msig_addr,
        value: TokenAmount::zero(),
        method: MsigMethod::SwapSigner as u64,
        params: serialize(&swap_params, "swap params").unwrap(),
    };

    // proposal from non swapped goes ok, txnid = 1
    apply_ok(
        v,
        &bob,
        &msig_addr,
        &TokenAmount::zero(),
        MsigMethod::Propose as u64,
        Some(propose_swap_signer_params),
    );
    let approve_swap_signer_params = TxnIDParams { id: TxnID(1), proposal_hash: vec![] };
    apply_ok(
        v,
        &dinesh,
        &msig_addr,
        &TokenAmount::zero(),
        MsigMethod::Approve as u64,
        Some(approve_swap_signer_params),
    );
    let st: MsigState = get_state(v, &msig_addr).unwrap();
    assert_eq!(vec![bob, chuck, alice], st.signers);

    assert_invariants(v, &Policy::default(), None)
}

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
        &TokenAmount::zero(),
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

fn check_txs(v: &dyn VM, msig_addr: Address, mut expect_txns: Vec<(TxnID, Transaction)>) {
    let st: MsigState = get_state(v, &msig_addr).unwrap();
    let store = DynBlockstore::wrap(v.blockstore());
    let ptx = PendingTxnMap::load(&store, &st.pending_txs, PENDING_TXN_CONFIG, "pending").unwrap();
    let mut actual_txns = Vec::new();
    ptx.for_each(|k, txn: &Transaction| {
        actual_txns.push((k, txn.clone()));
        Ok(())
    })
    .unwrap();
    expect_txns.sort_by_key(|(TxnID(id), _txn)| (*id));
    actual_txns.sort_by_key(|(TxnID(id), _txn)| (*id));
    assert_eq!(expect_txns, actual_txns);
}
