//! TODO: remove tests for legacy behaviour by deleting this file:
//! https://github.com/filecoin-project/builtin-actors/issues/1389
//! For now these tests preserve the behaviour of deals that are already (and will continue to be) handled by cron
//! The test fixtures replicate this behaviour by adding them explicitly to the deal_op queue upon activation and setting
//! last_updated to the deal_start epoch.

use fil_actor_market::{next_update_epoch, State};
use fil_actors_runtime::network::EPOCHS_IN_DAY;
use fil_actors_runtime::runtime::{Runtime, RuntimePolicy};
use fil_actors_runtime::test_utils::*;
use fil_actors_runtime::{BURNT_FUNDS_ACTOR_ADDR, EPOCHS_IN_YEAR};
use fvm_shared::address::Address;
use fvm_shared::clock::ChainEpoch;
use fvm_shared::econ::TokenAmount;
use fvm_shared::error::ExitCode;
use fvm_shared::METHOD_SEND;
use regex::Regex;

use num_traits::Zero;

mod harness;

use harness::*;

#[test]
fn slash_a_deal_and_make_payment_for_another_deal_in_the_same_epoch() {
    let start_epoch = ChainEpoch::from(50);
    let end_epoch = start_epoch + 200 * EPOCHS_IN_DAY;
    let sector_expiry = end_epoch + 100;
    let sector_1 = 7;

    let rt = setup();

    let (deal_id1, d1) = publish_and_activate_deal_legacy(
        &rt,
        CLIENT_ADDR,
        &MinerAddresses::default(),
        sector_1,
        start_epoch,
        end_epoch,
        0,
        sector_expiry,
    );

    let (deal_id2, _) = publish_and_activate_deal_legacy(
        &rt,
        CLIENT_ADDR,
        &MinerAddresses::default(),
        sector_1 + 1,
        start_epoch + 1,
        end_epoch + 1,
        0,
        sector_expiry,
    );

    // slash deal1
    let slash_epoch = process_epoch(start_epoch, deal_id2) + ChainEpoch::from(100);
    rt.set_epoch(slash_epoch);
    terminate_deals(&rt, PROVIDER_ADDR, &[sector_1], &[deal_id1]);
    cron_tick(&rt);

    assert_deal_deleted(&rt, deal_id1, &d1, sector_1, true);
    let s2 = get_deal_state(&rt, deal_id2);
    assert_eq!(slash_epoch, s2.last_updated_epoch);
    check_state(&rt);
}

#[test]
// TODO: remove tests for legacy behaviour: https://github.com/filecoin-project/builtin-actors/issues/1389
fn crontick_for_a_deal_at_its_start_epoch_results_in_zero_payment_and_no_slashing() {
    let start_epoch = ChainEpoch::from(50);
    let end_epoch = start_epoch + 200 * EPOCHS_IN_DAY;
    let sector_expiry = end_epoch + 100;
    let sector_number = 7;

    // set start epoch to coincide with processing (0 + 0 % 2880 = 0)
    let start_epoch = 0;
    let rt = setup();
    let (deal_id, _) = publish_and_activate_deal_legacy(
        &rt,
        CLIENT_ADDR,
        &MinerAddresses::default(),
        sector_number,
        start_epoch,
        end_epoch,
        0,
        sector_expiry,
    );

    // move the current epoch to processing epoch
    let current = process_epoch(start_epoch, deal_id);
    rt.set_epoch(current);
    let (pay, slashed) =
        cron_tick_and_assert_balances(&rt, CLIENT_ADDR, PROVIDER_ADDR, current, deal_id);
    assert_eq!(TokenAmount::zero(), pay);
    assert_eq!(TokenAmount::zero(), slashed);

    // deal proposal and state should NOT be deleted
    get_deal_proposal(&rt, deal_id);
    get_deal_state(&rt, deal_id);
    check_state(&rt);
}

// TODO: remove tests for legacy behaviour: https://github.com/filecoin-project/builtin-actors/issues/1389
#[test]
fn settling_deal_fails_when_deal_update_epoch_is_in_the_future() {
    let start_epoch = 50;
    let end_epoch = start_epoch + 200 * EPOCHS_IN_DAY;
    let sector_expiry = end_epoch + 100;
    let sector_number = 7;

    let rt = setup();

    let (deal_id, _) = publish_and_activate_deal_legacy(
        &rt,
        CLIENT_ADDR,
        &MinerAddresses::default(),
        sector_number,
        start_epoch,
        end_epoch,
        0,
        sector_expiry,
    );

    // move the current epoch such that the deal's last updated field is set to the start epoch of the deal
    // and the next tick for it is scheduled at the endepoch.
    rt.set_epoch(process_epoch(start_epoch, deal_id));
    cron_tick(&rt);

    // update last updated to some time in the future (breaks state invariants)
    update_last_updated(&rt, deal_id, end_epoch + 1000);

    // set current epoch of the deal to the end epoch so it's picked up for "processing" in the next cron tick.
    rt.set_epoch(end_epoch);
    expect_abort(ExitCode::USR_ILLEGAL_STATE, cron_tick_raw(&rt));
    let ret = settle_deal_payments(&rt, MinerAddresses::default().provider, &[deal_id], &[], &[]);
    assert_eq!(ret.results.codes(), &[ExitCode::USR_ILLEGAL_STATE]);

    check_state_with_expected(
        &rt,
        &[Regex::new("deal \\d+ last updated epoch \\d+ after current \\d+").unwrap()],
    );
}

#[test]
fn cron_reschedules_update_to_new_period() {
    let start_epoch = ChainEpoch::from(1);
    let end_epoch = start_epoch + 200 * EPOCHS_IN_DAY;
    let sector_number = 7;

    // Publish a deal
    let rt = setup();
    let (deal_id, _) = publish_and_activate_deal_legacy(
        &rt,
        CLIENT_ADDR,
        &MinerAddresses::default(),
        sector_number,
        start_epoch,
        end_epoch,
        0,
        end_epoch,
    );
    let update_interval = rt.policy().deal_updates_interval;

    // Hack state to move the scheduled update to some off-policy epoch.
    // This simulates there having been a prior policy that put it here, but now
    // the policy has changed.
    let mut st: State = rt.get_state();
    let expected_epoch = next_update_epoch(deal_id, update_interval, start_epoch);
    let misscheduled_epoch = expected_epoch + 42;
    st.remove_deals_by_epoch(rt.store(), &[expected_epoch]).unwrap();
    st.put_deals_by_epoch(rt.store(), &[(misscheduled_epoch, deal_id)]).unwrap();
    rt.replace_state(&st);

    let curr_epoch = rt.set_epoch(misscheduled_epoch);
    cron_tick(&rt);

    let st: State = rt.get_state();
    let expected_epoch = next_update_epoch(deal_id, update_interval, curr_epoch + 1);
    assert_ne!(expected_epoch, curr_epoch);
    assert_ne!(expected_epoch, misscheduled_epoch + update_interval);
    let found = st.get_deals_for_epoch(rt.store(), expected_epoch).unwrap();
    assert_eq!([deal_id][..], found[..]);
}

#[test]
fn cron_reschedules_update_to_new_period_boundary() {
    let start_epoch = ChainEpoch::from(1);
    let end_epoch = start_epoch + 200 * EPOCHS_IN_DAY;
    let sector_number = 7;

    // Publish a deal
    let rt = setup();
    let (deal_id, _) = publish_and_activate_deal_legacy(
        &rt,
        CLIENT_ADDR,
        &MinerAddresses::default(),
        sector_number,
        start_epoch,
        end_epoch,
        0,
        end_epoch,
    );
    let update_interval = rt.policy().deal_updates_interval;

    // Hack state to move the scheduled update.
    let mut st: State = rt.get_state();
    let expected_epoch = next_update_epoch(deal_id, update_interval, start_epoch);
    // Schedule the update exactly where the current policy would have put it anyway,
    // next time round (as if an old policy had an interval that was a multiple of the current one).
    // We can confirm it's rescheduled to the next period rather than left behind.
    let misscheduled_epoch = expected_epoch + update_interval;
    st.remove_deals_by_epoch(rt.store(), &[expected_epoch]).unwrap();
    st.put_deals_by_epoch(rt.store(), &[(misscheduled_epoch, deal_id)]).unwrap();
    rt.replace_state(&st);

    let curr_epoch = rt.set_epoch(misscheduled_epoch);
    cron_tick(&rt);

    let st: State = rt.get_state();
    let expected_epoch = next_update_epoch(deal_id, update_interval, curr_epoch + 1);
    assert_ne!(expected_epoch, curr_epoch);
    // For all other mis-schedulings, these would be asserted non-equal, but
    // for this case we expect a perfect increase of one update interval.
    assert_eq!(expected_epoch, misscheduled_epoch + update_interval);
    let found = st.get_deals_for_epoch(rt.store(), expected_epoch).unwrap();
    assert_eq!([deal_id][..], found[..]);
}

#[test]
fn cron_reschedules_many_updates() {
    let start_epoch = ChainEpoch::from(10);
    let end_epoch = start_epoch + 200 * EPOCHS_IN_DAY;
    let sector_expiry = start_epoch + 5 * EPOCHS_IN_YEAR;
    let sector_number = 7;
    // Set a short update interval so we can generate scheduling collisions.
    let update_interval = 100;

    // Publish a deal
    let mut rt = setup();
    rt.policy.deal_updates_interval = update_interval;
    let deal_count = 2 * update_interval;
    for i in 0..deal_count {
        publish_and_activate_deal_legacy(
            &rt,
            CLIENT_ADDR,
            &MinerAddresses::default(),
            sector_number,
            start_epoch,
            end_epoch + i,
            0,
            sector_expiry,
        );
    }

    let st: State = rt.get_state();
    // Confirm two deals are scheduled for each epoch from start_epoch.
    let first_updates = st.get_deals_for_epoch(rt.store(), start_epoch).unwrap();
    for epoch in start_epoch..(start_epoch + update_interval) {
        assert_eq!(2, st.get_deals_for_epoch(rt.store(), epoch).unwrap().len());
    }

    rt.set_epoch(start_epoch);
    cron_tick(&rt);

    let st: State = rt.get_state();
    // Two deals removed from start_epoch
    assert_eq!(0, st.get_deals_for_epoch(rt.store(), start_epoch).unwrap().len());

    // Same two deals scheduled one interval later
    let rescheduled = st.get_deals_for_epoch(rt.store(), start_epoch + update_interval).unwrap();
    assert_eq!(first_updates, rescheduled);

    for epoch in (start_epoch + 1)..(start_epoch + update_interval) {
        rt.set_epoch(epoch);
        cron_tick(&rt);
        let st: State = rt.get_state();
        assert_eq!(2, st.get_deals_for_epoch(rt.store(), epoch + update_interval).unwrap().len());
    }
}

#[test]
fn locked_fund_tracking_states() {
    // This test logic depends on fragile assumptions about how deal IDs are scheduled
    // for periodic updates.
    let p1 = Address::new_id(201);
    let p2 = Address::new_id(202);
    let p3 = Address::new_id(203);

    let c1 = Address::new_id(104);
    let c2 = Address::new_id(105);
    let c3 = Address::new_id(106);

    let m1 = MinerAddresses {
        owner: OWNER_ADDR,
        worker: WORKER_ADDR,
        provider: p1,
        control: vec![CONTROL_ADDR],
    };
    let m2 = MinerAddresses {
        owner: OWNER_ADDR,
        worker: WORKER_ADDR,
        provider: p2,
        control: vec![CONTROL_ADDR],
    };
    let m3 = MinerAddresses {
        owner: OWNER_ADDR,
        worker: WORKER_ADDR,
        provider: p3,
        control: vec![CONTROL_ADDR],
    };

    let start_epoch = ChainEpoch::from(2880);
    let end_epoch = start_epoch + 200 * EPOCHS_IN_DAY;
    let sector_expiry = end_epoch + 400;
    let sector_number = 7;

    let rt = setup();
    rt.actor_code_cids.borrow_mut().insert(p1, *MINER_ACTOR_CODE_ID);
    rt.actor_code_cids.borrow_mut().insert(c1, *ACCOUNT_ACTOR_CODE_ID);
    let st: State = rt.get_state();

    // assert values are zero
    assert!(st.total_client_locked_collateral.is_zero());
    assert!(st.total_provider_locked_collateral.is_zero());
    assert!(st.total_client_storage_fee.is_zero());

    // Publish deal1, deal2, and deal3 with different client and provider
    let (deal_id1, d1) = generate_and_publish_deal(&rt, c1, &m1, start_epoch, end_epoch);

    let (deal_id2, d2) = generate_and_publish_deal(&rt, c2, &m2, start_epoch, end_epoch);

    let (deal_id3, d3) = generate_and_publish_deal(&rt, c3, &m3, start_epoch, end_epoch);

    let csf = d1.total_storage_fee() + d2.total_storage_fee() + d3.total_storage_fee();
    let plc = &d1.provider_collateral + d2.provider_collateral + &d3.provider_collateral;
    let clc = d1.client_collateral + d2.client_collateral + &d3.client_collateral;

    assert_locked_fund_states(&rt, csf.clone(), plc.clone(), clc.clone());

    // activation doesn't change anything
    let curr = rt.set_epoch(start_epoch - 1);
    activate_deals_legacy(&rt, sector_expiry, p1, curr, sector_number, &[deal_id1]);
    activate_deals_legacy(&rt, sector_expiry, p2, curr, sector_number, &[deal_id2]);

    assert_locked_fund_states(&rt, csf.clone(), plc.clone(), clc.clone());

    // make payment for p1 and p2, p3 times out as it has not been activated
    let curr = rt.set_epoch(process_epoch(start_epoch, deal_id3));
    let last_payment_epoch = curr;
    rt.expect_send_simple(
        BURNT_FUNDS_ACTOR_ADDR,
        METHOD_SEND,
        None,
        d3.provider_collateral.clone(),
        None,
        ExitCode::OK,
    );
    cron_tick(&rt);
    let duration = curr - start_epoch;
    let payment: TokenAmount = 2 * &d1.storage_price_per_epoch * duration;
    let mut csf = (csf - payment) - d3.total_storage_fee();
    let mut plc = plc - d3.provider_collateral;
    let mut clc = clc - d3.client_collateral;
    assert_locked_fund_states(&rt, csf.clone(), plc.clone(), clc.clone());

    // Advance to just before the process epochs for deal 1 & 2, nothing changes before that.
    let curr = rt.set_epoch(process_epoch(curr, deal_id1) - 1);
    cron_tick(&rt);
    assert_locked_fund_states(&rt, csf.clone(), plc.clone(), clc.clone());

    // one more round of payment for deal1 and deal2
    let curr = rt.set_epoch(process_epoch(curr, deal_id2));
    let duration = curr - last_payment_epoch;
    let payment = 2 * d1.storage_price_per_epoch * duration;
    csf -= payment;
    cron_tick(&rt);
    assert_locked_fund_states(&rt, csf.clone(), plc.clone(), clc.clone());

    // slash deal1
    rt.set_epoch(curr + 1);
    terminate_deals(&rt, m1.provider, &[sector_number], &[deal_id1]);

    // cron tick to slash deal1 and expire deal2
    rt.set_epoch(end_epoch);
    csf = TokenAmount::zero();
    clc = TokenAmount::zero();
    plc = TokenAmount::zero();

    expect_emitted(&rt, "deal-completed", deal_id2, d2.client.id().unwrap(), p2.id().unwrap());

    cron_tick(&rt);
    assert_locked_fund_states(&rt, csf, plc, clc);
    check_state(&rt);
}

fn assert_locked_fund_states(
    rt: &MockRuntime,
    storage_fee: TokenAmount,
    provider_collateral: TokenAmount,
    client_collateral: TokenAmount,
) {
    let st: State = rt.get_state();

    assert_eq!(client_collateral, st.total_client_locked_collateral);
    assert_eq!(provider_collateral, st.total_provider_locked_collateral);
    assert_eq!(storage_fee, st.total_client_storage_fee);
}
