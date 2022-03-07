use fil_actors_runtime::test_utils::*;

mod util;

#[test]
fn test_can_set_peer_id() {
    let mut rt = MockRuntime::default();
    let h = util::ActorHarness::new(0);

    h.construct_and_verify(&mut rt);
    h.set_peer_id(&mut rt, vec![1, 2, 3]);

    // TODO check state invariants
}

#[test]
fn test_can_clear_peer_id() {}

#[test]
fn test_cant_set_large_peer_id() {}

#[test]
fn can_set_multiaddrs() {}

#[test]
fn can_set_multiple_multiaddrs() {}

#[test]
fn can_set_clear_multiaddrs() {}

#[test]
fn cant_set_empty_multiaddrs() {}

#[test]
fn cant_set_large_multiaddrs() {}
