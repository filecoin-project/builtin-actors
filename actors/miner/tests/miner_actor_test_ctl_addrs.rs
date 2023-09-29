use fil_actors_runtime::test_utils::*;
use fvm_shared::econ::TokenAmount;

mod util;

#[test]
fn test_control_addrs() {
    let mut rt = MockRuntime::default();
    rt.policy.new_miner_deposit = TokenAmount::default();

    let h = util::ActorHarness::new(0);

    h.construct_and_verify(&rt);

    let (owner, worker, control_addrs) = h.get_control_addresses(&rt);
    assert_eq!(h.owner, owner);
    assert_eq!(h.worker, worker);
    assert_eq!(h.control_addrs, control_addrs);

    h.check_state(&rt);
}
