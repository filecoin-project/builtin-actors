use fil_actor_multisig::{
    Actor as MultisigActor, ConstructorParams, Method, State, Transaction, TxnID, SIGNERS_MAX,
};
use fil_actors_runtime::test_utils::*;
use fil_actors_runtime::{INIT_ACTOR_ADDR, SYSTEM_ACTOR_ADDR};
use fvm_ipld_encoding::RawBytes;
use fvm_shared::address::Address;
use fvm_shared::econ::TokenAmount;
use fvm_shared::error::ExitCode;
use fvm_shared::METHOD_SEND;

mod util;

fn construct_runtime(receiver: Address) -> MockRuntime {
    MockRuntime {
        receiver: receiver,
        caller: *SYSTEM_ACTOR_ADDR,
        caller_type: *SYSTEM_ACTOR_CODE_ID,
        ..Default::default()
    }
}

// Constructor

#[test]
fn test_construction_fail_to_construct_multisig_actor_with_0_signers() {
    let msig = Address::new_id(1000);
    let mut rt = construct_runtime(msig);
    let zero_signer_params = ConstructorParams {
        signers: Vec::new(),
        num_approvals_threshold: 1,
        unlock_duration: 1,
        start_epoch: 0,
    };
    rt.expect_validate_caller_addr(vec![*INIT_ACTOR_ADDR]);
    rt.set_caller(*INIT_ACTOR_CODE_ID, *INIT_ACTOR_ADDR);

    expect_abort(
        ExitCode::ErrIllegalArgument,
        rt.call::<MultisigActor>(
            Method::Constructor as u64,
            &RawBytes::serialize(&zero_signer_params).unwrap(),
        ),
    );
    rt.verify();
}

#[test]
fn test_construction_fail_to_construct_multisig_with_more_than_max_signers() {
    let msig = Address::new_id(1000);
    let mut rt = construct_runtime(msig);
    let mut signers = Vec::new();
    let mut i: u64 = 0;
    while i <= SIGNERS_MAX as u64 {
        signers.push(Address::new_id(i + 1000));
        i = i + 1;
    }
    let over_max_signers_params = ConstructorParams {
        signers: signers,
        num_approvals_threshold: 1,
        unlock_duration: 1,
        start_epoch: 0,
    };
    rt.expect_validate_caller_addr(vec![*INIT_ACTOR_ADDR]);
    rt.set_caller(*INIT_ACTOR_CODE_ID, *INIT_ACTOR_ADDR);
    expect_abort(
        ExitCode::ErrIllegalArgument,
        rt.call::<MultisigActor>(
            Method::Constructor as u64,
            &RawBytes::serialize(&over_max_signers_params).unwrap(),
        ),
    );
    rt.verify();
}

// Propose

#[test]
fn test_simple_propose() {
    let msig = Address::new_id(1000);
    let mut rt = construct_runtime(msig);
    let h = util::ActorHarness::new();

    let anne = Address::new_id(101);
    let bob = Address::new_id(102);
    let chuck = Address::new_id(103);
    let no_unlock_duration = 0;
    let start_epoch = 0;
    let signers = vec![anne, bob];

    let send_value = TokenAmount::from(10u8);
    h.construct_and_verify(&mut rt, 2, no_unlock_duration, start_epoch, signers);
    rt.set_caller(*ACCOUNT_ACTOR_CODE_ID, anne);
    h.propose_ok(&mut rt, chuck, send_value.clone(), METHOD_SEND, RawBytes::default());
    let txn0 = Transaction {
        to: chuck,
        value: send_value,
        method: METHOD_SEND,
        params: RawBytes::default(),
        approved: vec![anne],
    };
    let expect_txns = vec![(TxnID(0), txn0)];
    h.assert_transactions(&rt, expect_txns);
}

// AddSigner

#[test]
fn test_happy_path_add_signer() {
    let msig = Address::new_id(100);
    let anne = Address::new_id(101);
    let bob = Address::new_id(102);
    let chuck = Address::new_id(103);
    let mut rt = construct_runtime(msig);
    let initial_signers = vec![anne, bob];
    let initial_approvals: u64 = 2;

    // construct the multisig actor and add id addrs to runtime
    let h = util::ActorHarness::new();
    h.construct_and_verify(&mut rt, initial_approvals, 0, 0, initial_signers);
    // add the signer with the expected params
    rt.set_caller(*MULTISIG_ACTOR_CODE_ID, msig);
    expect_ok(h.add_signer(&mut rt, chuck, false));

    // check that the state matches what we expect
    let expected_signers = vec![anne, bob, chuck];
    let expected_approvals = initial_approvals;

    let st = rt.get_state::<State>().unwrap();
    assert_eq!(expected_signers, st.signers);
    assert_eq!(expected_approvals, st.num_approvals_threshold);
}

// Approve

#[test]
fn test_approve_simple_propose_and_approval() {
    // setup rt
    let msig = Address::new_id(100);
    let anne = Address::new_id(101);
    let bob = Address::new_id(102);
    let chuck = Address::new_id(103);

    let mut rt = construct_runtime(msig);
    let h = util::ActorHarness::new();
    // construct msig
    let signers = vec![anne, bob];

    h.construct_and_verify(&mut rt, 2, 0, 0, signers);

    let fake_params = RawBytes::from(vec![1, 2, 3, 4]);
    let fake_method = 42;
    let fake_ret = RawBytes::from(vec![4, 3, 2, 1]);
    let send_value = TokenAmount::from(10u8);
    rt.set_caller(*ACCOUNT_ACTOR_CODE_ID, anne);
    let proposal_hash =
        h.propose_ok(&mut rt, chuck, send_value.clone(), fake_method, fake_params.clone());

    // assert txn
    let expect_txn = Transaction {
        to: chuck,
        value: send_value.clone(),
        method: fake_method,
        params: fake_params.clone(),
        approved: vec![anne],
    };
    h.assert_transactions(&rt, vec![(TxnID(0), expect_txn)]);

    // approval
    rt.set_balance(send_value.clone());
    rt.set_caller(*ACCOUNT_ACTOR_CODE_ID, bob);
    rt.expect_send(chuck, fake_method, fake_params, send_value, fake_ret, ExitCode::Ok);
    h.approve_ok(&mut rt, TxnID(0), proposal_hash);
    h.assert_transactions(&rt, vec![]);
}
