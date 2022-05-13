#![allow(clippy::all)]
use fil_actors_runtime::test_utils::*;

mod util;

#[test]
fn test_control_addrs() {
    let mut rt = MockRuntime::default();
    let h = util::ActorHarness::new(0);

    h.construct_and_verify(&mut rt);

    let (owner, worker, control_addrs) = h.get_control_addresses(&mut rt);
    assert_eq!(h.owner, owner);
    assert_eq!(h.worker, worker);
    assert_eq!(h.control_addrs, control_addrs);

    util::check_state_invariants(&rt);
}
