use fvm_shared::clock::ChainEpoch;

mod util;
use util::*;

const PERIOD_OFFSET: ChainEpoch = 100;

#[test]
fn cron_enrolls_on_precommit_prove_commits_and_continues_enrolling() {
    let mut h = ActorHarness::new(PERIOD_OFFSET);
    let rt = h.new_runtime();
    rt.set_balance(BIG_BALANCE.clone());
    h.construct_and_verify(&rt);

    let cron_ctrl = CronControl::default();
    let long_expiration = 500;

    cron_ctrl.require_cron_inactive(&h, &rt);
    let sectors = h.commit_and_prove_sectors(&rt, 1, long_expiration, vec![], true);
    cron_ctrl.require_cron_active(&h, &rt);

    // advance cron to activate power.
    h.advance_and_submit_posts(&rt, &sectors);
    // advance 499 days of deadline (1 before expiration occurrs)
    // this asserts that cron continues to enroll within advanceAndSubmitPoSt
    for _ in 0..499 {
        h.advance_and_submit_posts(&rt, &sectors);
    }
    h.check_state(&rt);
    let st = h.get_state(&rt);
    assert!(st.deadline_cron_active);
}

#[test]
fn cron_enrolls_on_precommit_expires_on_pcd_expiration_re_enrolls_on_new_precommit_immediately() {
    let h = ActorHarness::new(PERIOD_OFFSET);
    let rt = h.new_runtime();
    rt.set_balance(BIG_BALANCE.clone());
    let epoch = PERIOD_OFFSET + 1;
    rt.set_epoch(epoch);
    h.construct_and_verify(&rt);
    let mut cron_ctrl = CronControl::default();

    let epoch = cron_ctrl.pre_commit_start_cron_expire_stop_cron(&h, &rt, epoch);
    cron_ctrl.pre_commit_to_start_cron(&h, &rt, epoch);
}

#[test]
fn cron_enrolls_on_precommit_expires_on_pcd_expiration_re_enrolls_on_new_precommit_after_falling_out_of_date(
) {
    let h = ActorHarness::new(PERIOD_OFFSET);
    let rt = h.new_runtime();
    rt.set_balance(BIG_BALANCE.clone());
    let mut epoch = PERIOD_OFFSET + 1;
    rt.set_epoch(epoch);
    h.construct_and_verify(&rt);
    let mut cron_ctrl = CronControl::default();

    epoch = cron_ctrl.pre_commit_start_cron_expire_stop_cron(&h, &rt, epoch);
    // Advance some epochs to fall several pp out of date, then precommit again reenrolling cron
    epoch += 200 * rt.policy.wpost_proving_period;
    epoch = cron_ctrl.pre_commit_start_cron_expire_stop_cron(&h, &rt, epoch);
    // Stay within the same deadline but advance an epoch
    epoch += 1;
    cron_ctrl.pre_commit_to_start_cron(&h, &rt, epoch);
}

#[test]
fn enroll_pcd_expire_re_enroll_x_3() {
    let h = ActorHarness::new(PERIOD_OFFSET);
    let rt = h.new_runtime();
    rt.set_balance(BIG_BALANCE.clone());
    let mut epoch = PERIOD_OFFSET + 1;
    rt.set_epoch(epoch);
    h.construct_and_verify(&rt);
    let mut cron_ctrl = CronControl::default();
    for _ in 1..3 {
        epoch = cron_ctrl.pre_commit_start_cron_expire_stop_cron(&h, &rt, epoch) + 42;
    }
}
