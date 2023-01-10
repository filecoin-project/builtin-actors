use fil_actors_runtime::test_utils::*;

use fil_actor_miner::{Actor, ChangeMultiaddrsParams, GetMultiaddrsReturn, Method};
use fvm_ipld_encoding::ipld_block::IpldBlock;
use fvm_ipld_encoding::BytesDe;
use fvm_shared::error::ExitCode;

mod util;

#[test]
fn test_can_set_peer_id() {
    let mut rt = MockRuntime::default();
    let h = util::ActorHarness::new(0);

    h.construct_and_verify(&mut rt);
    h.set_peer_id(&mut rt, vec![1, 2, 3]);

    h.check_state(&rt);
}

#[test]
fn test_can_clear_peer_id() {
    let mut rt = MockRuntime::default();
    let h = util::ActorHarness::new(0);

    h.construct_and_verify(&mut rt);
    h.set_peer_id(&mut rt, vec![]);

    h.check_state(&rt);
}

#[test]
fn test_cant_set_large_peer_id() {
    let mut rt = MockRuntime::default();
    let h = util::ActorHarness::new(0);
    let peer_id = vec![0; rt.policy.max_peer_id_length + 1];

    h.construct_and_verify(&mut rt);
    h.set_peer_id_fail(&mut rt, peer_id);

    h.check_state(&rt);
}

#[test]
fn can_set_multiaddrs() {
    let mut rt = MockRuntime::default();
    let h = util::ActorHarness::new(0);

    h.construct_and_verify(&mut rt);
    h.set_multiaddr(&mut rt, vec![BytesDe(vec![1, 3, 3, 7])]);

    h.check_state(&rt);
}

#[test]
fn can_set_multiple_multiaddrs() {
    let mut rt = MockRuntime::default();
    let h = util::ActorHarness::new(0);

    h.construct_and_verify(&mut rt);
    h.set_multiaddr(&mut rt, vec![BytesDe(vec![1, 3, 3, 7]), BytesDe(vec![2, 4, 4, 8])]);

    h.check_state(&rt);
}

#[test]
fn can_set_clear_multiaddrs() {
    let mut rt = MockRuntime::default();
    let h = util::ActorHarness::new(0);

    h.construct_and_verify(&mut rt);
    h.set_multiaddr(&mut rt, vec![]);

    h.check_state(&rt);
}

#[test]
fn cant_set_empty_multiaddrs() {
    let mut rt = MockRuntime::default();
    let h = util::ActorHarness::new(0);

    h.construct_and_verify(&mut rt);
    h.set_multiaddr_fail(&mut rt, vec![BytesDe(vec![])]);

    h.check_state(&rt);
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

    h.check_state(&rt);
}

#[test]
fn get_and_change_multiaddrs_restricted_correctly() {
    let mut rt = MockRuntime::default();
    let h = util::ActorHarness::new(0);

    h.construct_and_verify(&mut rt);

    let new_multiaddrs = vec![BytesDe(vec![1, 3, 3, 7])];

    let params = IpldBlock::serialize_cbor(&ChangeMultiaddrsParams {
        new_multi_addrs: new_multiaddrs.clone(),
    })
    .unwrap();

    rt.set_caller(*EVM_ACTOR_CODE_ID, h.worker);

    // fail to call the unexported setter

    expect_abort_contains_message(
        ExitCode::USR_FORBIDDEN,
        "must be built-in",
        rt.call::<Actor>(Method::ChangeMultiaddrs as u64, params.clone()),
    );

    // call the exported setter

    rt.expect_validate_caller_addr(h.caller_addrs());

    rt.call::<Actor>(Method::ChangeMultiaddrsExported as u64, params).unwrap();
    rt.verify();

    // call the exported getter

    rt.expect_validate_caller_any();
    let ret: GetMultiaddrsReturn = rt
        .call::<Actor>(Method::GetMultiaddrsExported as u64, None)
        .unwrap()
        .deserialize()
        .unwrap();
    rt.verify();

    assert_eq!(new_multiaddrs, ret.multi_addrs);

    h.check_state(&rt);
}
