// Copyright 2019-2022 ChainSafe Systems
// SPDX-License-Identifier: Apache-2.0, MIT

use fil_actors_runtime::network::EPOCHS_IN_DAY;
use fil_actors_runtime::runtime::Policy;
use fvm_shared::clock::ChainEpoch;

mod harness;
use harness::*;

const START_EPOCH: ChainEpoch = 50;
const DURATION_EPOCHS: ChainEpoch = 200 * EPOCHS_IN_DAY;
const END_EPOCH: ChainEpoch = START_EPOCH + DURATION_EPOCHS;

#[test]
fn deal_is_correctly_processed_if_first_cron_after_expiry() {
    let rt = setup();
    let deal_id = publish_and_activate_deal(
        &rt,
        CLIENT_ADDR,
        &MinerAddresses::default(),
        START_EPOCH,
        END_EPOCH,
        0,
        END_EPOCH,
    );
    let deal_proposal = get_deal_proposal(&rt, deal_id);

    // move the current epoch to startEpoch
    let current = START_EPOCH;
    rt.set_epoch(current);
    let (pay, slashed) =
        cron_tick_and_assert_balances(&rt, CLIENT_ADDR, PROVIDER_ADDR, current, deal_id);
    assert!(pay.is_zero());
    assert!(slashed.is_zero());
    // assert deal exists
    let _found = get_deal_proposal(&rt, deal_id);

    // move the epoch to endEpoch+5(anything greater than endEpoch)
    // total payment = (end - start)
    let current = END_EPOCH + 5;
    rt.set_epoch(current);
    let (pay, slashed) =
        cron_tick_and_assert_balances(&rt, CLIENT_ADDR, PROVIDER_ADDR, current, deal_id);
    let duration = END_EPOCH - START_EPOCH;
    assert_eq!(duration * &deal_proposal.storage_price_per_epoch, pay);
    assert!(slashed.is_zero());

    // deal should be deleted as it should have expired
    assert_deal_deleted(&rt, deal_id, deal_proposal);
    check_state(&rt);
}

#[test]
fn regular_payments_till_deal_expires_and_then_locked_funds_are_unlocked() {
    let start_epoch = Policy::default().deal_updates_interval;
    let end_epoch = start_epoch + DURATION_EPOCHS;
    let rt = setup();
    let deal_id = publish_and_activate_deal(
        &rt,
        CLIENT_ADDR,
        &MinerAddresses::default(),
        start_epoch,
        end_epoch,
        0,
        end_epoch,
    );
    // The logic of this test relies on deal ID == 0 so that it's scheduled for
    // updated in the 0th epoch of every interval, and the start epoch being the same.
    assert_eq!(0, deal_id);
    let deal_proposal = get_deal_proposal(&rt, deal_id);

    // move the current epoch to startEpoch + 5 so payment is made
    // this skip of 5 epochs is unrealistic, but later demonstrates that the re-scheduled
    // epoch distribution is robust to this.
    let current = start_epoch + 5;
    rt.set_epoch(current);
    // assert payment
    assert!(!deal_proposal.storage_price_per_epoch.is_zero());
    let (pay, slashed) =
        cron_tick_and_assert_balances(&rt, CLIENT_ADDR, PROVIDER_ADDR, current, deal_id);
    assert_eq!(5 * &deal_proposal.storage_price_per_epoch, pay);
    assert!(slashed.is_zero());

    // Setting the current epoch to anything less than next schedule wont make any payment.
    // Note the re-processing is scheduled for start+interval, not current+interval
    // (which differs because we skipped some epochs).
    let current = start_epoch + Policy::default().deal_updates_interval - 1;
    rt.set_epoch(current);
    cron_tick_no_change(&rt, CLIENT_ADDR, PROVIDER_ADDR);

    // however setting the current epoch to next schedule will make the payment
    let current = current + 1;
    rt.set_epoch(current);
    let duration = Policy::default().deal_updates_interval - 5;
    let (pay, slashed) =
        cron_tick_and_assert_balances(&rt, CLIENT_ADDR, PROVIDER_ADDR, current, deal_id);
    assert_eq!(duration * &deal_proposal.storage_price_per_epoch, pay);
    assert!(slashed.is_zero());

    // a second cron tick for the same epoch should not change anything
    cron_tick_no_change(&rt, CLIENT_ADDR, PROVIDER_ADDR);

    // next epoch schedule
    let current = current + Policy::default().deal_updates_interval;
    rt.set_epoch(current);
    let duration = Policy::default().deal_updates_interval;
    let (pay, slashed) =
        cron_tick_and_assert_balances(&rt, CLIENT_ADDR, PROVIDER_ADDR, current, deal_id);
    assert_eq!(duration * &deal_proposal.storage_price_per_epoch, pay);
    assert!(slashed.is_zero());

    // setting epoch to greater than end will expire the deal, make the payment and unlock all funds
    let duration = end_epoch - current;
    let current = end_epoch + 300;
    rt.set_epoch(current);
    let (pay, slashed) =
        cron_tick_and_assert_balances(&rt, CLIENT_ADDR, PROVIDER_ADDR, current, deal_id);
    assert_eq!(duration * &deal_proposal.storage_price_per_epoch, pay);
    assert!(slashed.is_zero());

    // deal should be deleted as it should have expired
    assert_deal_deleted(&rt, deal_id, deal_proposal);
    check_state(&rt);
}

#[test]
fn payment_for_a_deal_if_deal_is_already_expired_before_a_cron_tick() {
    let start = 5;
    let end = start + 200 * EPOCHS_IN_DAY;

    let rt = setup();
    let deal_id =
        publish_and_activate_deal(&rt, CLIENT_ADDR, &MinerAddresses::default(), start, end, 0, end);
    let deal_proposal = get_deal_proposal(&rt, deal_id);

    let current = end + 25;
    rt.set_epoch(current);

    let (pay, slashed) =
        cron_tick_and_assert_balances(&rt, CLIENT_ADDR, PROVIDER_ADDR, current, deal_id);
    assert_eq!((end - start) * &deal_proposal.storage_price_per_epoch, pay);
    assert!(slashed.is_zero());

    assert_deal_deleted(&rt, deal_id, deal_proposal);

    // running cron tick again doesn't do anything
    cron_tick_no_change(&rt, CLIENT_ADDR, PROVIDER_ADDR);
    check_state(&rt);
}

#[test]
fn expired_deal_should_unlock_the_remaining_client_and_provider_locked_balance_after_payment_and_deal_should_be_deleted(
) {
    let rt = setup();
    let deal_id = publish_and_activate_deal(
        &rt,
        CLIENT_ADDR,
        &MinerAddresses::default(),
        START_EPOCH,
        END_EPOCH,
        0,
        END_EPOCH,
    );
    let deal_proposal = get_deal_proposal(&rt, deal_id);

    let c_escrow = get_balance(&rt, &CLIENT_ADDR).balance;
    let p_escrow = get_balance(&rt, &PROVIDER_ADDR).balance;

    // move the current epoch so that deal is expired
    rt.set_epoch(END_EPOCH + 1000);
    cron_tick(&rt);

    // assert balances
    let payment = deal_proposal.total_storage_fee();

    let client_acct = get_balance(&rt, &CLIENT_ADDR);
    assert_eq!(c_escrow - &payment, client_acct.balance);
    assert!(client_acct.locked.is_zero());

    let provider_acct = get_balance(&rt, &PROVIDER_ADDR);
    assert_eq!(p_escrow + &payment, provider_acct.balance);
    assert!(provider_acct.locked.is_zero());

    // deal should be deleted
    assert_deal_deleted(&rt, deal_id, deal_proposal);
    check_state(&rt);
}

#[test]
fn all_payments_are_made_for_a_deal_client_withdraws_collateral_and_client_account_is_removed() {
    let rt = setup();
    let deal_id = publish_and_activate_deal(
        &rt,
        CLIENT_ADDR,
        &MinerAddresses::default(),
        START_EPOCH,
        END_EPOCH,
        0,
        END_EPOCH,
    );
    let deal_proposal = get_deal_proposal(&rt, deal_id);

    // move the current epoch so that deal is expired
    rt.set_epoch(END_EPOCH + 100);
    cron_tick(&rt);
    assert_eq!(deal_proposal.client_collateral, get_balance(&rt, &CLIENT_ADDR).balance);

    // client withdraws collateral -> account should be removed as it now has zero balance
    withdraw_client_balance(
        &rt,
        deal_proposal.client_collateral.clone(),
        deal_proposal.client_collateral,
        CLIENT_ADDR,
    );
    assert_account_zero(&rt, CLIENT_ADDR);
    check_state(&rt);
}
