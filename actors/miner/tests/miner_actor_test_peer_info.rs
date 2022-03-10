use fil_actors_runtime::test_utils::*;

use fvm_shared::encoding::BytesDe;

mod util;

#[test]
fn test_can_set_peer_id() {
    let mut rt = MockRuntime::default();
    let h = util::ActorHarness::new(0);

    h.construct_and_verify(&mut rt);
    h.set_peer_id(&mut rt, vec![1, 2, 3]);

    util::check_state_invariants(&rt);
}

#[test]
fn test_can_clear_peer_id() {
    let mut rt = MockRuntime::default();
    let h = util::ActorHarness::new(0);

    h.construct_and_verify(&mut rt);
    h.set_peer_id(&mut rt, vec![]);

    util::check_state_invariants(&rt);
}

#[test]
fn test_cant_set_large_peer_id() {
    let mut rt = MockRuntime::default();
    let h = util::ActorHarness::new(0);
    let peer_id = vec![0; rt.policy.max_peer_id_length + 1];

    h.construct_and_verify(&mut rt);
    h.set_peer_id_fail(&mut rt, peer_id);

    util::check_state_invariants(&rt);
}

#[test]
fn can_set_multiaddrs() {
    let mut rt = MockRuntime::default();
    let h = util::ActorHarness::new(0);

    h.construct_and_verify(&mut rt);
    h.set_multiaddr(&mut rt, vec![BytesDe(vec![1, 3, 3, 7])]);

    util::check_state_invariants(&rt);
}

#[test]
fn can_set_multiple_multiaddrs() {
    let mut rt = MockRuntime::default();
    let h = util::ActorHarness::new(0);

    h.construct_and_verify(&mut rt);
    h.set_multiaddr(&mut rt, vec![BytesDe(vec![1, 3, 3, 7]), BytesDe(vec![2, 4, 4, 8])]);

    util::check_state_invariants(&rt);
}

#[test]
fn can_set_clear_multiaddrs() {
    let mut rt = MockRuntime::default();
    let h = util::ActorHarness::new(0);

    h.construct_and_verify(&mut rt);
    h.set_multiaddr(&mut rt, vec![]);

    util::check_state_invariants(&rt);
}

#[test]
fn cant_set_empty_multiaddrs() {
    let mut rt = MockRuntime::default();
    let h = util::ActorHarness::new(0);

    h.construct_and_verify(&mut rt);
    h.set_multiaddr_fail(&mut rt, vec![BytesDe(vec![])]);

    util::check_state_invariants(&rt);
}

#[test]
fn cant_set_large_multiaddrs() {
    let mut rt = MockRuntime::default();
    let h = util::ActorHarness::new(0);

    let mut maddrs = Vec::new();
    for i in 0..100 {
        maddrs.push(BytesDe(vec![
            i + 1,
            i + 2,
            i + 3,
            i + 4,
            i + 5,
            i + 6,
            i + 7,
            i + 8,
            i + 9,
            i + 10,
            i + 11,
        ]));
    }

    h.construct_and_verify(&mut rt);
    h.set_multiaddr_fail(&mut rt, maddrs);

    util::check_state_invariants(&rt);
}
