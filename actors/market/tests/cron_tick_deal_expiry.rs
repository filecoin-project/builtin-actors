// Copyright 2019-2022 ChainSafe Systems
// SPDX-License-Identifier: Apache-2.0, MIT

use fil_actors_runtime::network::EPOCHS_IN_DAY;
use fil_actors_runtime::runtime::Policy;
use fvm_shared::clock::ChainEpoch;

use num_traits::Zero;

mod harness;
use harness::*;

const START_EPOCH: ChainEpoch = 50;
const END_EPOCH: ChainEpoch = START_EPOCH + 200 * EPOCHS_IN_DAY;
const SECTOR_EXPIRY: ChainEpoch = END_EPOCH + 400;

#[test]
fn deal_is_correctly_processed_twice_in_the_same_crontick() {
    let mut rt = setup();
    let deal_id = publish_and_activate_deal(
        &mut rt,
        CLIENT_ADDR,
        &MinerAddresses::default(),
        START_EPOCH,
        END_EPOCH,
        0,
        SECTOR_EXPIRY,
    );
    let deal_proposal = get_deal_proposal(&mut rt, deal_id);

    // move the current epoch to startEpoch and scheduled next epoch at endepoch -1
    let current = START_EPOCH;
    rt.set_epoch(current);
    let (pay, slashed) =
        cron_tick_and_assert_balances(&mut rt, CLIENT_ADDR, PROVIDER_ADDR, current, deal_id);
    assert!(pay.is_zero());
    assert!(slashed.is_zero());
    // assert deal exists
    let _found = get_deal_proposal(&mut rt, deal_id);

    // move the epoch to endEpoch+5(anything greater than endEpoch), so deal is first processed at endEpoch - 1 AND then at it's end epoch
    // total payment = (end - start)
    let current = END_EPOCH + 5;
    rt.set_epoch(current);
    let (pay, slashed) =
        cron_tick_and_assert_balances(&mut rt, CLIENT_ADDR, PROVIDER_ADDR, current, deal_id);
    let duration = END_EPOCH - START_EPOCH;
    assert_eq!(duration * &deal_proposal.storage_price_per_epoch, pay);
    assert!(slashed.is_zero());

    // deal should be deleted as it should have expired
    assert_deal_deleted(&mut rt, deal_id, deal_proposal);
    check_state(&rt);
}

#[test]
fn regular_payments_till_deal_expires_and_then_locked_funds_are_unlocked() {
    // start epoch should equal first processing epoch for logic to work
    // 2880 + 0 % 2880 = 2880
    const START_EPOCH: ChainEpoch = EPOCHS_IN_DAY;
    let mut rt = setup();
    let deal_id = publish_and_activate_deal(
        &mut rt,
        CLIENT_ADDR,
        &MinerAddresses::default(),
        START_EPOCH,
        END_EPOCH,
        0,
        SECTOR_EXPIRY,
    );
    let deal_proposal = get_deal_proposal(&mut rt, deal_id);

    // move the current epoch to startEpoch + 5 so payment is made
    let current = START_EPOCH + 5;
    rt.set_epoch(current);
    // assert payment
    let (pay, slashed) =
        cron_tick_and_assert_balances(&mut rt, CLIENT_ADDR, PROVIDER_ADDR, current, deal_id);
    assert_eq!(5 * &deal_proposal.storage_price_per_epoch, pay);
    assert!(slashed.is_zero());

    // Setting the current epoch to anything less than next schedule wont make any payment
    let current = current + Policy::default().deal_updates_interval - 1;
    rt.set_epoch(current);
    cron_tick_no_change(&mut rt, CLIENT_ADDR, PROVIDER_ADDR);

    // however setting the current epoch to next schedle will make the payment
    let current = current + 1;
    rt.set_epoch(current);
    let duration = Policy::default().deal_updates_interval;
    let (pay, slashed) =
        cron_tick_and_assert_balances(&mut rt, CLIENT_ADDR, PROVIDER_ADDR, current, deal_id);
    assert_eq!(duration * &deal_proposal.storage_price_per_epoch, pay);
    assert!(slashed.is_zero());

    // a second cron tick for the same epoch should not change anything
    cron_tick_no_change(&mut rt, CLIENT_ADDR, PROVIDER_ADDR);

    // next epoch schedule
    let current = current + Policy::default().deal_updates_interval;
    rt.set_epoch(current);
    let duration = Policy::default().deal_updates_interval;
    let (pay, slashed) =
        cron_tick_and_assert_balances(&mut rt, CLIENT_ADDR, PROVIDER_ADDR, current, deal_id);
    assert_eq!(duration * &deal_proposal.storage_price_per_epoch, pay);
    assert!(slashed.is_zero());

    // setting epoch to greater than end will expire the deal, make the payment and unlock all funds
    let duration = END_EPOCH - current;
    let current = END_EPOCH + 300;
    rt.set_epoch(current);
    let (pay, slashed) =
        cron_tick_and_assert_balances(&mut rt, CLIENT_ADDR, PROVIDER_ADDR, current, deal_id);
    assert_eq!(duration * &deal_proposal.storage_price_per_epoch, pay);
    assert!(slashed.is_zero());

    // deal should be deleted as it should have expired
    assert_deal_deleted(&mut rt, deal_id, deal_proposal);
    check_state(&rt);
}

#[test]
fn payment_for_a_deal_if_deal_is_already_expired_before_a_cron_tick() {
    let start = 5;
    let end = start + 200 * EPOCHS_IN_DAY;

    let mut rt = setup();
    let deal_id = publish_and_activate_deal(
        &mut rt,
        CLIENT_ADDR,
        &MinerAddresses::default(),
        start,
        end,
        0,
        SECTOR_EXPIRY,
    );
    let deal_proposal = get_deal_proposal(&mut rt, deal_id);

    let current = end + 25;
    rt.set_epoch(current);

    let (pay, slashed) =
        cron_tick_and_assert_balances(&mut rt, CLIENT_ADDR, PROVIDER_ADDR, current, deal_id);
    assert_eq!((end - start) * &deal_proposal.storage_price_per_epoch, pay);
    assert!(slashed.is_zero());

    assert_deal_deleted(&mut rt, deal_id, deal_proposal);

    // running cron tick again doesn't do anything
    cron_tick_no_change(&mut rt, CLIENT_ADDR, PROVIDER_ADDR);
    check_state(&rt);
}

#[test]
fn expired_deal_should_unlock_the_remaining_client_and_provider_locked_balance_after_payment_and_deal_should_be_deleted(
) {
    let mut rt = setup();
    let deal_id = publish_and_activate_deal(
        &mut rt,
        CLIENT_ADDR,
        &MinerAddresses::default(),
        START_EPOCH,
        END_EPOCH,
        0,
        SECTOR_EXPIRY,
    );
    let deal_proposal = get_deal_proposal(&mut rt, deal_id);

    let c_escrow = get_escrow_balance(&rt, &CLIENT_ADDR).unwrap();
    let p_escrow = get_escrow_balance(&rt, &PROVIDER_ADDR).unwrap();

    // move the current epoch so that deal is expired
    rt.set_epoch(END_EPOCH + 1000);
    cron_tick(&mut rt);

    // assert balances
    let payment = deal_proposal.total_storage_fee();

    assert_eq!(c_escrow - &payment, get_escrow_balance(&rt, &CLIENT_ADDR).unwrap());
    assert!(get_locked_balance(&mut rt, CLIENT_ADDR).is_zero());

    assert_eq!(p_escrow + &payment, get_escrow_balance(&rt, &PROVIDER_ADDR).unwrap());
    assert!(get_locked_balance(&mut rt, PROVIDER_ADDR).is_zero());

    // deal should be deleted
    assert_deal_deleted(&mut rt, deal_id, deal_proposal);
    check_state(&rt);
}

#[test]
fn all_payments_are_made_for_a_deal_client_withdraws_collateral_and_client_account_is_removed() {
    let mut rt = setup();
    let deal_id = publish_and_activate_deal(
        &mut rt,
        CLIENT_ADDR,
        &MinerAddresses::default(),
        START_EPOCH,
        END_EPOCH,
        0,
        SECTOR_EXPIRY,
    );
    let deal_proposal = get_deal_proposal(&mut rt, deal_id);

    // move the current epoch so that deal is expired
    rt.set_epoch(END_EPOCH + 100);
    cron_tick(&mut rt);
    assert_eq!(deal_proposal.client_collateral, get_escrow_balance(&rt, &CLIENT_ADDR).unwrap());

    // client withdraws collateral -> account should be removed as it now has zero balance
    withdraw_client_balance(
        &mut rt,
        deal_proposal.client_collateral.clone(),
        deal_proposal.client_collateral,
        CLIENT_ADDR,
    );
    assert_account_zero(&mut rt, CLIENT_ADDR);
    check_state(&rt);
}
