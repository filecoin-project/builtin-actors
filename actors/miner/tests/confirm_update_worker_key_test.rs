use fil_actor_miner::State;
use fil_actors_runtime::{
    runtime::{Runtime, RuntimePolicy},
    test_utils::MockRuntime,
};
use fvm_shared::{address::Address, econ::TokenAmount};

mod util;
use util::*;

const NEW_WORKER: Address = Address::new_id(999);

fn setup() -> (ActorHarness, MockRuntime) {
    let big_balance = 20u128.pow(23);
    let period_offset = 100;
    let current_epoch = 5;

    let h = ActorHarness::new(period_offset);
    let mut rt = h.new_runtime();
    h.construct_and_verify(&mut rt);
    rt.balance.replace(TokenAmount::from(big_balance));
    rt.set_epoch(current_epoch);

    (h, rt)
}

#[test]
fn successfully_changes_the_worker_address() {
    let (h, mut rt) = setup();

    let effective_epoch = rt.epoch + rt.policy().worker_key_change_delay;
    h.change_worker_address(&mut rt, NEW_WORKER, h.control_addrs.clone()).unwrap();

    // confirm at effective epoch
    rt.set_epoch(effective_epoch);
    h.confirm_update_worker_key(&mut rt).unwrap();

    let state: State = rt.get_state();
    let info = state.get_info(rt.store()).unwrap();

    assert_eq!(info.worker, NEW_WORKER);
    assert!(info.pending_worker_key.is_none());

    check_state_invariants(&rt);
}

#[test]
fn does_nothing_before_the_effective_date() {
    let (h, mut rt) = setup();

    let effective_epoch = rt.epoch + rt.policy().worker_key_change_delay;
    h.change_worker_address(&mut rt, NEW_WORKER, h.control_addrs.clone()).unwrap();

    // confirm right before the effective epoch
    rt.set_epoch(effective_epoch - 1);
    h.confirm_update_worker_key(&mut rt).unwrap();

    let state: State = rt.get_state();
    let info = state.get_info(rt.store()).unwrap();

    assert_eq!(info.worker, h.worker);
    assert!(info.pending_worker_key.is_some());

    check_state_invariants(&rt);
}

#[test]
fn does_nothing_when_no_update_is_set() {
    let (h, mut rt) = setup();

    h.confirm_update_worker_key(&mut rt).unwrap();

    let state: State = rt.get_state();
    let info = state.get_info(rt.store()).unwrap();

    assert_eq!(info.worker, h.worker);
    assert!(info.pending_worker_key.is_none());

    check_state_invariants(&rt);
}
