use fil_actor_miner::State;
use fil_actors_runtime::{
    runtime::{Runtime, RuntimePolicy},
    test_utils::MockRuntime,
};
use fvm_shared::address::Address;

mod util;
use util::*;

const NEW_WORKER: Address = Address::new_id(999);

fn setup() -> (ActorHarness, MockRuntime) {
    let period_offset = 100;
    let current_epoch = 5;

    let h = ActorHarness::new(period_offset);
    let rt = h.new_runtime();
    h.construct_and_verify(&rt);
    rt.balance.replace(BIG_BALANCE.clone());
    rt.set_epoch(current_epoch);

    (h, rt)
}

#[test]
fn successfully_changes_the_worker_address() {
    let (h, rt) = setup();

    let effective_epoch = *rt.epoch.borrow() + rt.policy().worker_key_change_delay;
    h.change_worker_address(&rt, NEW_WORKER, h.control_addrs.clone()).unwrap();

    // confirm at effective epoch
    rt.set_epoch(effective_epoch);
    h.confirm_change_worker_address(&rt).unwrap();

    let state: State = rt.get_state();
    let info = state.get_info(rt.store()).unwrap();

    assert_eq!(info.worker, NEW_WORKER);
    assert!(info.pending_worker_key.is_none());

    h.check_state(&rt);
}

#[test]
fn does_nothing_before_the_effective_date() {
    let (h, rt) = setup();

    let effective_epoch = *rt.epoch.borrow() + rt.policy().worker_key_change_delay;
    h.change_worker_address(&rt, NEW_WORKER, h.control_addrs.clone()).unwrap();

    // confirm right before the effective epoch
    rt.set_epoch(effective_epoch - 1);
    h.confirm_change_worker_address(&rt).unwrap();

    let state: State = rt.get_state();
    let info = state.get_info(rt.store()).unwrap();

    assert_eq!(info.worker, h.worker);

    let pending_worker_key = info.pending_worker_key.unwrap();
    assert_eq!(pending_worker_key.new_worker, NEW_WORKER);
    assert_eq!(pending_worker_key.effective_at, effective_epoch);

    h.check_state(&rt);
}

#[test]
fn does_nothing_when_no_update_is_set() {
    let (h, rt) = setup();

    h.confirm_change_worker_address(&rt).unwrap();

    let state: State = rt.get_state();
    let info = state.get_info(rt.store()).unwrap();

    assert_eq!(info.worker, h.worker);
    assert!(info.pending_worker_key.is_none());

    h.check_state(&rt);
}
