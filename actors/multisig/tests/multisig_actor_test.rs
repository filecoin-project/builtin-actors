// use cid::Cid;
use fil_actor_multisig::{Actor as MultisigActor, ConstructorParams, Method};
use fil_actors_runtime::test_utils::*;
use fil_actors_runtime::{INIT_ACTOR_ADDR, SYSTEM_ACTOR_ADDR};
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
    let ret = rt.call::<MultisigActor>(
        Method::Constructor as u64,
        &RawBytes::serialize(&zero_signer_params).unwrap(),
    );
    rt.verify();
    let error = ret.expect_err("constructor with no signers should have failed");
    let error_exit_code = error.exit_code();

    assert_eq!(
        error_exit_code,
        ExitCode::ErrIllegalArgument,
        "Exit Code that is returned is not ErrIllegalArgument"
    );
}
