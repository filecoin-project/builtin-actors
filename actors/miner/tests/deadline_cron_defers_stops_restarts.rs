use fvm_shared::clock::ChainEpoch;
use fvm_shared::econ::TokenAmount;

mod util;
use util::*;

const PERIOD_OFFSET: ChainEpoch = 100;
const BIG_BALANCE: u128 = 1_000_000_000_000_000_000_000_000u128;

#[test]
fn cron_enrolls_on_precommit_prove_commits_and_continues_enrolling() {
    let mut h = ActorHarness::new(PERIOD_OFFSET);
    let mut rt = h.new_runtime();
    rt.set_balance(TokenAmount::from(BIG_BALANCE));
    h.construct_and_verify(&mut rt);

    let cron_ctrl = CronControl { pre_commit_num: 0 };
    let long_expiration = 500;

    cron_ctrl.require_cron_inactive(&h, &rt);
    let sectors = h.commit_and_prove_sectors(&mut rt, 1, long_expiration, vec![], true);
    cron_ctrl.require_cron_active(&h, &rt);

    // advance cron to activate power.
    h.advance_and_submit_posts(&mut rt, &sectors);
    // advance 499 days of deadline (1 before expiration occurrs)
    // this asserts that cron continues to enroll within advanceAndSubmitPoSt
    for _ in 0..499 {
        h.advance_and_submit_posts(&mut rt, &sectors);
    }
    check_state_invariants(&rt);
    let st = h.get_state(&rt);
    assert!(st.deadline_cron_active);
}

#[test]
fn cron_enrolls_on_precommit_expires_on_pcd_expiration_re_enrolls_on_new_precommit_immediately() {
    let h = ActorHarness::new(PERIOD_OFFSET);
    let mut rt = h.new_runtime();
    rt.set_balance(TokenAmount::from(BIG_BALANCE));
    let epoch = PERIOD_OFFSET + 1;
    rt.set_epoch(epoch);
    h.construct_and_verify(&mut rt);
    let mut cron_ctrl = CronControl::default();

    let epoch = cron_ctrl.pre_commit_start_cron_expire_stop_cron(&h, &mut rt, epoch);
    cron_ctrl.pre_commit_to_start_cron(&h, &mut rt, epoch);
}

#[test]
fn cron_enrolls_on_precommit_expires_on_pcd_expiration_re_enrolls_on_new_precommit_after_falling_out_of_date(
) {
    let h = ActorHarness::new(PERIOD_OFFSET);
    let mut rt = h.new_runtime();
    rt.set_balance(TokenAmount::from(BIG_BALANCE));
    let mut epoch = PERIOD_OFFSET + 1;
    rt.set_epoch(epoch);
    h.construct_and_verify(&mut rt);
    let mut cron_ctrl = CronControl::default();

    epoch = cron_ctrl.pre_commit_start_cron_expire_stop_cron(&h, &mut rt, epoch);
    // Advance some epochs to fall several pp out of date, then precommit again reenrolling cron
    epoch = epoch + 200 * rt.policy.wpost_proving_period;
    epoch = cron_ctrl.pre_commit_start_cron_expire_stop_cron(&h, &mut rt, epoch);
    // Stay within the same deadline but advance an epoch
    epoch = epoch + 1;
    cron_ctrl.pre_commit_to_start_cron(&h, &mut rt, epoch);
}

#[test]
fn enroll_pcd_expire_re_enroll_x_3() {
    let h = ActorHarness::new(PERIOD_OFFSET);
    let mut rt = h.new_runtime();
    rt.set_balance(TokenAmount::from(BIG_BALANCE));
    let mut epoch = PERIOD_OFFSET + 1;
    rt.set_epoch(epoch);
    h.construct_and_verify(&mut rt);
    let mut cron_ctrl = CronControl::default();
    for _ in 1..3 {
        epoch = cron_ctrl.pre_commit_start_cron_expire_stop_cron(&h, &mut rt, epoch) + 42;
    }
}
