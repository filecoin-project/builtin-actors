use fil_actors_runtime::test_utils::MockRuntime;
use fvm_shared::econ::TokenAmount;

mod util;
use util::*;

fn setup() -> (ActorHarness, MockRuntime) {
    let big_balance = 20u128.pow(23);
    let period_offset = 100;
    let precommit_epoch = 1;

    let h = ActorHarness::new(period_offset);
    let mut rt = h.new_runtime();
    h.construct_and_verify(&mut rt);
    rt.balance.replace(TokenAmount::from(big_balance));
    rt.set_epoch(precommit_epoch);

    (h, rt)
}

#[test]
fn successfully_change_peer_id() {
    let (h, mut rt) = setup();
    let new_pid = b"cthulhu".to_vec();
    h.change_peer_id(&mut rt, new_pid);

    check_state_invariants(&rt);
}
