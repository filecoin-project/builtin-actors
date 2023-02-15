use fil_actor_miner::{Actor, ChangePeerIDParams, GetPeerIDReturn, Method};
use fil_actors_runtime::test_utils::{
    expect_abort_contains_message, MockRuntime, EVM_ACTOR_CODE_ID,
};
use fvm_ipld_encoding::ipld_block::IpldBlock;
use fvm_shared::error::ExitCode;

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

#[test]
fn change_peer_id_restricted_correctly() {
    let (h, mut rt) = setup();

    let new_id = b"cthulhu".to_vec();

    let params = IpldBlock::serialize_cbor(&ChangePeerIDParams { new_id: new_id.clone() }).unwrap();

    rt.set_caller(*EVM_ACTOR_CODE_ID, h.worker);

    // fail to call the unexported setter

    expect_abort_contains_message(
        ExitCode::USR_FORBIDDEN,
        "must be built-in",
        rt.call::<Actor>(Method::ChangePeerID as u64, params.clone()),
    );

    // call the exported setter

    rt.expect_validate_caller_addr(h.caller_addrs());

    rt.call::<Actor>(Method::ChangePeerIDExported as u64, params).unwrap();

    // call the exported getter

    rt.expect_validate_caller_any();
    let ret: GetPeerIDReturn = rt
        .call::<Actor>(Method::GetPeerIDExported as u64, None)
        .unwrap()
        .unwrap()
        .deserialize()
        .unwrap();
    rt.verify();

    assert_eq!(new_id, ret.peer_id);

    h.check_state(&rt);
}
