use export_macro::vm_test;
use fil_actor_init::ExecReturn;
use fil_actor_multisig::{
    Method as MsigMethod, ProposeParams, State as MsigState, TxnIDParams, 
    PENDING_TXN_CONFIG, PendingTxnMap, Transaction, TxnID,
};
use fil_actors_runtime::cbor::serialize;
use fil_actors_runtime::runtime::Policy;
use fil_actors_runtime::test_utils::*;
use fil_actors_runtime::INIT_ACTOR_ADDR;
use fvm_ipld_encoding::RawBytes;
use fvm_shared::METHOD_SEND;
use fvm_shared::address::Address;
use fvm_shared::bigint::Zero;
use fvm_shared::econ::TokenAmount;
use vm_api::VM;
use vm_api::util::{apply_ok, get_state, DynBlockstore};

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
pub fn nested_multisig_test(v: &dyn VM) {
    // Create regular accounts for signers
    let addrs = create_accounts(v, 5, &TokenAmount::from_whole(10_000));
    let alice = addrs[0];
    let bob = addrs[1];
    let charlie = addrs[2];
    let dave = addrs[3];
    let recipient = addrs[4];
    
    // Create an inner multisig with Alice and Bob as signers, threshold 2
    let inner_msig_addr = create_msig(v, &[alice, bob], 2);
    
    // Fund the inner multisig
    apply_ok(
        v,
        &alice,
        &inner_msig_addr,
        &TokenAmount::from_whole(100),
        METHOD_SEND,
        None::<RawBytes>,
    );
    
    // Create the main multisig with inner multisig and Charlie as signers, threshold 2
    let main_msig_addr = create_msig(v, &[inner_msig_addr, charlie], 2);
    
    // Fund the main multisig
    apply_ok(
        v,
        &alice,
        &main_msig_addr,
        &TokenAmount::from_whole(1000),
        METHOD_SEND,
        None::<RawBytes>,
    );
    
    // Verify the main multisig has the correct signers
    let main_st: MsigState = get_state(v, &main_msig_addr).unwrap();
    assert_eq!(vec![inner_msig_addr, charlie], main_st.signers);
    assert_eq!(2, main_st.num_approvals_threshold);
    
    // Now let's test sending money from the main multisig
    // Step 1: Inner multisig proposes to the main multisig
    let send_amount = TokenAmount::from_whole(50);
    let send_to_recipient_params = ProposeParams {
        to: recipient,
        value: send_amount.clone(),
        method: METHOD_SEND,
        params: RawBytes::default(),
    };
    
    // Alice proposes in the inner multisig to propose in the main multisig
    let propose_to_main_params = ProposeParams {
        to: main_msig_addr,
        value: TokenAmount::zero(),
        method: MsigMethod::Propose as u64,
        params: serialize(&send_to_recipient_params, "propose params").unwrap(),
    };
    
    apply_ok(
        v,
        &alice,
        &inner_msig_addr,
        &TokenAmount::from_atto(0),
        MsigMethod::Propose as u64,
        Some(propose_to_main_params),
    );
    
    // Bob approves in the inner multisig (this should execute the proposal to main multisig)
    let inner_approve_params = TxnIDParams {
        id: fil_actor_multisig::TxnID(0),
        proposal_hash: vec![], // hash optional
    };
    
    apply_ok(
        v,
        &bob,
        &inner_msig_addr,
        &TokenAmount::from_atto(0),
        MsigMethod::Approve as u64,
        Some(inner_approve_params),
    );
    
    // Now the inner multisig has proposed to the main multisig
    // Check that the main multisig has a pending transaction
    check_txs(v, main_msig_addr, vec![(TxnID(0), Transaction {
        to: recipient,
        value: send_amount.clone(),
        method: METHOD_SEND,
        params: RawBytes::default(),
        approved: vec![inner_msig_addr],
    })]);
    
    // Step 2: Charlie approves in the main multisig (this should execute the send)
    let recipient_balance_before = v.actor(&recipient).unwrap().balance;
    
    let main_approve_params = TxnIDParams {
        id: fil_actor_multisig::TxnID(0),
        proposal_hash: vec![], // hash optional
    };
    
    apply_ok(
        v,
        &charlie,
        &main_msig_addr,
        &TokenAmount::from_atto(0),
        MsigMethod::Approve as u64,
        Some(main_approve_params),
    );
    
    // Verify the recipient received the funds
    let recipient_balance_after = v.actor(&recipient).unwrap().balance;
    assert_eq!(
        recipient_balance_before + send_amount,
        recipient_balance_after,
        "Recipient should have received the funds"
    );
    
    // Verify the main multisig has no pending transactions
    check_txs(v, main_msig_addr, vec![]);
    
    assert_invariants(v, &Policy::default(), None);
}

/// Check that transactions match expected
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

#[vm_test]
pub fn nested_multisig_direct_proposal_test(v: &dyn VM) {
    // Test where Dave directly proposes to main multisig (without going through inner multisig)
    let addrs = create_accounts(v, 5, &TokenAmount::from_whole(10_000));
    let alice = addrs[0];
    let bob = addrs[1];
    let charlie = addrs[2];
    let dave = addrs[3];
    let recipient = addrs[4];
    
    // Create inner multisig
    let inner_msig_addr = create_msig(v, &[alice, bob], 2);
    
    // Create main multisig with inner multisig, Charlie, and Dave as signers, threshold 2
    let main_msig_addr = create_msig(v, &[inner_msig_addr, charlie, dave], 2);
    
    // Fund the main multisig
    apply_ok(
        v,
        &alice,
        &main_msig_addr,
        &TokenAmount::from_whole(1000),
        METHOD_SEND,
        None::<RawBytes>,
    );
    
    // Dave proposes directly to send funds
    let send_amount = TokenAmount::from_whole(25);
    let send_params = ProposeParams {
        to: recipient,
        value: send_amount.clone(),
        method: METHOD_SEND,
        params: RawBytes::default(),
    };
    
    apply_ok(
        v,
        &dave,
        &main_msig_addr,
        &TokenAmount::from_atto(0),
        MsigMethod::Propose as u64,
        Some(send_params),
    );
    
    // Charlie approves (should execute since threshold is 2)
    let approve_params = TxnIDParams {
        id: fil_actor_multisig::TxnID(0),
        proposal_hash: vec![],
    };
    
    let recipient_balance_before = v.actor(&recipient).unwrap().balance;
    
    apply_ok(
        v,
        &charlie,
        &main_msig_addr,
        &TokenAmount::from_atto(0),
        MsigMethod::Approve as u64,
        Some(approve_params),
    );
    
    // Verify the recipient received the funds
    let recipient_balance_after = v.actor(&recipient).unwrap().balance;
    assert_eq!(
        recipient_balance_before + send_amount,
        recipient_balance_after,
        "Recipient should have received the funds"
    );
    
    assert_invariants(v, &Policy::default(), None);
}