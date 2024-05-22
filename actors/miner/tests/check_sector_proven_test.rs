use fil_actors_runtime::test_utils::{expect_abort, MockRuntime};
use fvm_shared::error::ExitCode;

mod util;
use util::*;

fn setup() -> (ActorHarness, MockRuntime) {
    let period_offset = 100;

    let h = ActorHarness::new(period_offset);
    let rt = h.new_runtime();
    h.construct_and_verify(&rt);
    rt.balance.replace(BIG_BALANCE.clone());

    (h, rt)
}

#[test]
fn successfully_check_sector_is_proven() {
    let (mut h, rt) = setup();

    let sectors =
        h.commit_and_prove_sectors(&rt, 1, DEFAULT_SECTOR_EXPIRATION, vec![vec![10]], true);
    h.check_sector_proven(&rt, sectors[0].sector_number).unwrap();

    h.check_state(&rt);
}

#[test]
fn fails_if_sector_is_not_found() {
    let (h, rt) = setup();

    let result = h.check_sector_proven(&rt, 1);
    expect_abort(ExitCode::USR_NOT_FOUND, result);

    h.check_state(&rt);
}
