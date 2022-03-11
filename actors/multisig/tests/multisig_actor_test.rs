// use cid::Cid;
use fil_actor_multisig::{Actor as MultisigActor, ConstructorParams, Method, State, SIGNERS_MAX};
use fil_actors_runtime::test_utils::*;
use fil_actors_runtime::{INIT_ACTOR_ADDR, SYSTEM_ACTOR_ADDR};
use fvm_shared::address::Address;
// use fvm_shared::econ::TokenAmount;
use fvm_shared::encoding::RawBytes;
use fvm_shared::error::ExitCode;

mod util;

// use serde::Serialize;

<<<<<<< HEAD
fn construct_runtime(receiver: Address) -> MockRuntime {
    MockRuntime {
        receiver: receiver,
=======
fn construct_runtime(reciever: Address) -> MockRuntime {
    MockRuntime {
        receiver: reciever,
>>>>>>> 1805053 (TestAddSigners happy path test)
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
fn test_simple_propose () {
    let msig = Address::new_id(1000);
    let mut rt = construct_runtime(msig);
    let h = util::ActorHarness {};

    let anne = Address::new_id(101);
    let bob = Address::new_id(102);
    let chuck = Address::new_id(103);
    let no_unlock_duration = 0;
    let start_epoch = 0;
    let signers = vec![anne, bob];

    h.construct_and_verify(&mut rt, 2, no_unlock_duration, start_epoch, signers);
    rt.set_caller(*ACCOUNT_ACTOR_CODE_ID, anne);

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
    let h = util::ActorHarness {};
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
