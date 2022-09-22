use fil_actors_runtime::test_utils::MockRuntime;

mod util;
use util::*;

fn setup() -> (ActorHarness, MockRuntime) {
    let period_offset = 100;
    let precommit_epoch = 1;

    let h = ActorHarness::new(period_offset);
    let mut rt = h.new_runtime();
    h.construct_and_verify(&mut rt);
    rt.balance.replace(BIG_BALANCE.clone());
    rt.set_epoch(precommit_epoch);

    (h, rt)
}

#[test]
fn successfully_change_peer_id() {
    let (h, mut rt) = setup();
    let new_pid = b"cthulhu".to_vec();
    h.change_peer_id(&mut rt, new_pid);

    h.check_state(&rt);
}
