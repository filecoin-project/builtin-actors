use fil_actors_runtime::test_utils::{expect_abort, MockRuntime};
use fvm_shared::{econ::TokenAmount, error::ExitCode};

mod util;
use util::*;

fn setup() -> (ActorHarness, MockRuntime) {
    let big_balance = 20u128.pow(23);
    let period_offset = 100;

    let h = ActorHarness::new(period_offset);
    let mut rt = h.new_runtime();
    h.construct_and_verify(&mut rt);
    rt.balance.replace(TokenAmount::from(big_balance));

    (h, rt)
}

#[test]
fn successfully_check_sector_is_proven() {
    let (mut h, mut rt) = setup();

    let sectors = h.commit_and_prove_sectors(&mut rt, 1, DEFAULT_SECTOR_EXPIRATION, vec![10], true);
    h.check_sector_proven(&mut rt, sectors[0].sector_number).unwrap();

    check_state_invariants(&rt);
}

#[test]
fn fails_if_sector_is_not_found() {
    let (h, mut rt) = setup();

    let result = h.check_sector_proven(&mut rt, 1);
    expect_abort(ExitCode::USR_NOT_FOUND, result);

    check_state_invariants(&rt);
}
