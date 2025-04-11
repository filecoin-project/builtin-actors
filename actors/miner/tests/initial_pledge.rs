use fil_actor_miner::{Actor, InitialPledgeReturn, Method, State};
use fil_actors_runtime::test_utils::*;
use fvm_shared::econ::TokenAmount;
use util::{ActorHarness, DEFAULT_SECTOR_EXPIRATION};
mod util;

fn setup() -> (ActorHarness, MockRuntime) {
    let big_balance = 20u128.pow(23);
    let period_offset = 100;
    let precommit_epoch = 1;

    let h = ActorHarness::new(period_offset);
    let rt = h.new_runtime();
    h.construct_and_verify(&rt);
    rt.balance.replace(TokenAmount::from_atto(big_balance));
    rt.set_epoch(precommit_epoch);

    (h, rt)
}

#[test]
fn test_initial_pledge_method() {
    let (mut h, rt) = setup();
    let _sector_info =
        h.commit_and_prove_sectors(&rt, 1, DEFAULT_SECTOR_EXPIRATION, Vec::new(), true);

    let st: State = rt.get_state();

    rt.expect_validate_caller_any();
    let ret: InitialPledgeReturn = rt
        .call::<Actor>(Method::InitialPledgeExported as u64, None)
        .unwrap()
        .unwrap()
        .deserialize()
        .unwrap();
    rt.verify();

    assert!(st.initial_pledge.is_positive());
    assert_eq!(st.initial_pledge, ret.initial_pledge);

    h.check_state(&rt);
}
