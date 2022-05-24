use fil_actor_account::Method as AccountMethod;
use fil_actor_miner::{Actor, ChangeWorkerAddressParams, Method};
use fil_actors_runtime::{
    runtime::RuntimePolicy,
    test_utils::{
        expect_abort, expect_abort_contains_message, new_bls_addr, MockRuntime,
        ACCOUNT_ACTOR_CODE_ID, MINER_ACTOR_CODE_ID,
    },
};
use fvm_ipld_encoding::RawBytes;
use fvm_shared::{address::Address, econ::TokenAmount, error::ExitCode};

mod util;
use itertools::Itertools;
use num_traits::Zero;
use util::*;

fn setup() -> (ActorHarness, MockRuntime) {
    let big_balance = 20u128.pow(23);
    let period_offset = 100;

    let h = ActorHarness::new(period_offset);
    let mut rt = h.new_runtime();
    h.construct_and_verify(&mut rt);
    rt.balance.replace(TokenAmount::from(big_balance));

    (h, rt)
}

#[test]
fn successfully_change_only_the_worker_address() {
    let (h, mut rt) = setup();

    let original_control_addresses = h.control_addrs.clone();
    let new_worker = Address::new_id(999);

    // set epoch to something close to next deadline so first cron will be before effective date
    let current_epoch = 2970;
    rt.set_epoch(current_epoch);

    let effective_epoch = current_epoch + rt.policy().worker_key_change_delay;
    h.change_worker_address(&mut rt, new_worker, original_control_addresses.clone()).unwrap();

    // assert change has been made in state
    let pending_worker_key = h.get_info(&rt).pending_worker_key.unwrap();
    assert_eq!(pending_worker_key.new_worker, new_worker);
    assert_eq!(pending_worker_key.effective_at, effective_epoch);

    // change if current epoch is less than effective epoch
    let deadline = h.deadline(&rt);
    rt.set_epoch(deadline.period_end());

    let info = h.get_info(&rt);
    assert_eq!(info.pending_worker_key.unwrap().new_worker, new_worker);
    assert_eq!(h.worker, info.worker);

    // move to deadline containing effective epoch
    rt.set_epoch(effective_epoch);

    // enact worker change
    h.confirm_update_worker_key(&mut rt).unwrap();

    // assert address has changed
    let info = h.get_info(&rt);
    assert_eq!(new_worker, info.worker);

    // assert control addresses are unchanged
    assert!(!info.control_addresses.is_empty());
    assert_eq!(original_control_addresses, info.control_addresses);

    check_state_invariants(&rt);
}

#[test]
fn change_cannot_be_overridden() {
    let (h, mut rt) = setup();

    let original_control_addresses = h.control_addrs.clone();
    let (new_worker_1, new_worker_2) = (Address::new_id(999), Address::new_id(1023));

    // set epoch to something close to next deadline so first cron will be before effective date
    let current_epoch = 2970;
    rt.set_epoch(current_epoch);

    let effective_epoch = current_epoch + rt.policy().worker_key_change_delay;
    h.change_worker_address(&mut rt, new_worker_1, original_control_addresses.clone()).unwrap();

    // no change if current epoch is less than effective epoch
    let deadline = h.deadline(&rt);
    rt.set_epoch(deadline.period_end());

    // attempt to change address again
    h.change_worker_address(&mut rt, new_worker_2, original_control_addresses).unwrap();

    // assert change has not been modified
    let pending_worker_key = h.get_info(&rt).pending_worker_key.unwrap();
    assert_eq!(pending_worker_key.new_worker, new_worker_1);
    assert_eq!(pending_worker_key.effective_at, effective_epoch);

    rt.set_epoch(effective_epoch);
    h.confirm_update_worker_key(&mut rt).unwrap();

    // assert original change is effected
    assert_eq!(new_worker_1, h.get_info(&rt).worker);
    check_state_invariants(&rt);
}

#[test]
fn successfully_resolve_and_change_only_control_addresses() {
    let (h, mut rt) = setup();

    let (control_address_1, control_address_2) = (Address::new_id(555), Address::new_id(556));
    let control_address_2_non_id = new_bls_addr(42);
    rt.add_id_address(control_address_2_non_id, control_address_2);
    rt.set_address_actor_type(control_address_1, *ACCOUNT_ACTOR_CODE_ID);
    rt.set_address_actor_type(control_address_2, *ACCOUNT_ACTOR_CODE_ID);

    h.change_worker_address(&mut rt, h.worker, vec![control_address_1, control_address_2_non_id])
        .unwrap();

    // assert there is no worker change request and worker key is unchanged
    let info = h.get_info(&rt);
    assert_eq!(h.worker, info.worker);
    assert!(info.pending_worker_key.is_none());

    check_state_invariants(&rt);
}

#[test]
fn successfully_change_both_worker_and_control_addresses() {
    let (h, mut rt) = setup();

    let new_worker = Address::new_id(999);
    let (control_address_1, control_address_2) = (Address::new_id(5001), Address::new_id(5002));
    rt.set_address_actor_type(control_address_1, *ACCOUNT_ACTOR_CODE_ID);
    rt.set_address_actor_type(control_address_2, *ACCOUNT_ACTOR_CODE_ID);

    let current_epoch = 5;
    rt.set_epoch(current_epoch);
    let effective_epoch = current_epoch + rt.policy().worker_key_change_delay;
    h.change_worker_address(&mut rt, new_worker, vec![control_address_1, control_address_2])
        .unwrap();

    // set current epoch and update worker key
    rt.set_epoch(effective_epoch);
    h.confirm_update_worker_key(&mut rt).unwrap();

    // assert both worker and control addresses have changed
    let info = h.get_info(&rt);
    assert_eq!(info.control_addresses, vec![control_address_1, control_address_2]);
    assert_eq!(info.worker, new_worker);

    check_state_invariants(&rt);
}

#[test]
fn successfully_clear_all_control_addresses() {
    let (h, mut rt) = setup();

    h.change_worker_address(&mut rt, h.worker, Vec::new()).unwrap();

    // assert control addresses are cleared
    let info = h.get_info(&rt);
    assert!(info.control_addresses.is_empty());

    check_state_invariants(&rt);
}

#[test]
fn fails_if_control_addresses_length_exceeds_maximum_limit() {
    let (h, mut rt) = setup();

    let control_addresses =
        (0..=rt.policy().max_control_addresses as u64).map(Address::new_id).collect_vec();
    let result = h.change_worker_address(&mut rt, h.worker, control_addresses);
    expect_abort_contains_message(
        ExitCode::USR_ILLEGAL_ARGUMENT,
        "control addresses length",
        result,
    );

    check_state_invariants(&rt);
}

#[test]
fn fails_if_unable_to_resolve_control_address() {
    let (h, mut rt) = setup();

    let control_address = new_bls_addr(42);
    let result = h.change_worker_address(&mut rt, h.worker, vec![control_address]);
    expect_abort(ExitCode::USR_ILLEGAL_ARGUMENT, result);

    check_state_invariants(&rt);
}

#[test]
fn fails_if_unable_to_resolve_worker_address() {
    let (h, mut rt) = setup();

    let new_worker = new_bls_addr(42);
    let result = h.change_worker_address(&mut rt, new_worker, vec![]);
    expect_abort(ExitCode::USR_ILLEGAL_ARGUMENT, result);

    check_state_invariants(&rt);
}

#[test]
fn fails_if_worker_public_key_is_not_bls() {
    let (mut h, mut rt) = setup();

    let new_worker = Address::new_id(999);
    h.worker_key = Address::new_id(505);

    let result = h.change_worker_address(&mut rt, new_worker, vec![]);
    expect_abort(ExitCode::USR_ILLEGAL_ARGUMENT, result);

    check_state_invariants(&rt);
}

#[test]
fn fails_if_new_worker_address_does_not_have_a_code() {
    let (h, mut rt) = setup();

    let new_worker = Address::new_id(5001);

    rt.set_caller(*ACCOUNT_ACTOR_CODE_ID, h.owner);

    let params = ChangeWorkerAddressParams { new_worker, new_control_addresses: Vec::new() };
    let result =
        rt.call::<Actor>(Method::ChangeWorkerAddress as u64, &RawBytes::serialize(params).unwrap());
    expect_abort(ExitCode::USR_ILLEGAL_ARGUMENT, result);
    rt.verify();

    check_state_invariants(&rt);
}

#[test]
fn fails_if_new_worker_is_not_account_actor() {
    let (h, mut rt) = setup();

    let new_worker = Address::new_id(999);
    rt.set_address_actor_type(new_worker, *MINER_ACTOR_CODE_ID);
    rt.set_caller(*ACCOUNT_ACTOR_CODE_ID, h.owner);

    let params = ChangeWorkerAddressParams { new_worker, new_control_addresses: Vec::new() };
    let result =
        rt.call::<Actor>(Method::ChangeWorkerAddress as u64, &RawBytes::serialize(params).unwrap());
    expect_abort(ExitCode::USR_ILLEGAL_ARGUMENT, result);
    rt.verify();

    check_state_invariants(&rt);
}

#[test]
fn fails_when_caller_is_not_the_owner() {
    let (h, mut rt) = setup();

    let new_worker = Address::new_id(999);
    rt.set_address_actor_type(new_worker, *ACCOUNT_ACTOR_CODE_ID);
    rt.expect_validate_caller_addr(vec![h.owner]);
    rt.expect_send(
        new_worker,
        AccountMethod::PubkeyAddress as u64,
        RawBytes::default(),
        TokenAmount::zero(),
        RawBytes::serialize(h.worker_key).unwrap(),
        ExitCode::OK,
    );
    rt.set_caller(*ACCOUNT_ACTOR_CODE_ID, h.worker);

    let params = ChangeWorkerAddressParams { new_worker, new_control_addresses: Vec::new() };
    let result =
        rt.call::<Actor>(Method::ChangeWorkerAddress as u64, &RawBytes::serialize(params).unwrap());
    expect_abort(ExitCode::USR_FORBIDDEN, result);
    rt.verify();

    check_state_invariants(&rt);
}
