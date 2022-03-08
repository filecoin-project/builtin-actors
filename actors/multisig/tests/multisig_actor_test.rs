// use cid::Cid;
use fil_actor_multisig::{Actor as MultisigActor, ConstructorParams, Method, SIGNERS_MAX};
use fil_actors_runtime::test_utils::*;
use fil_actors_runtime::{ActorError, INIT_ACTOR_ADDR, SYSTEM_ACTOR_ADDR};
use fvm_shared::address::Address;
// use fvm_shared::econ::TokenAmount;
use fvm_shared::encoding::RawBytes;
use fvm_shared::error::ExitCode;
// use serde::Serialize;

fn construct_runtime() -> MockRuntime {
    MockRuntime {
        receiver: Address::new_id(1000),
        caller: *SYSTEM_ACTOR_ADDR,
        caller_type: *SYSTEM_ACTOR_CODE_ID,
        ..Default::default()
    }
}

#[test]
fn test_construction_fail_to_construct_multisig_actor_with_0_signers() {
    let mut rt = construct_runtime();
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
        || -> Result<RawBytes, ActorError> {
            rt.call::<MultisigActor>(
                Method::Constructor as u64,
                &RawBytes::serialize(&zero_signer_params).unwrap(),
            )
        },
    );
    rt.verify();
}

#[test]
fn test_construction_fail_to_construct_multisig_with_more_than_max_signers() {
    let mut rt = construct_runtime();
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
        || -> Result<RawBytes, ActorError> {
            rt.call::<MultisigActor>(
                Method::Constructor as u64,
                &RawBytes::serialize(&over_max_signers_params).unwrap(),
            )
        },
    );
    rt.verify();
}
