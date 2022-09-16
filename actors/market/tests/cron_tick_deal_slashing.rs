// Copyright 2019-2022 ChainSafe Systems
// SPDX-License-Identifier: Apache-2.0, MIT

use fil_actors_runtime::network::EPOCHS_IN_DAY;
use fil_actors_runtime::runtime::Policy;
use fil_actors_runtime::BURNT_FUNDS_ACTOR_ADDR;
use fvm_ipld_encoding::RawBytes;
use fvm_shared::clock::ChainEpoch;
use fvm_shared::econ::TokenAmount;
use fvm_shared::error::ExitCode;
use fvm_shared::METHOD_SEND;

use num_traits::Zero;

mod harness;
use harness::*;

const SECTOR_EXPIRY: ChainEpoch = 400 + 200 * EPOCHS_IN_DAY;

#[test]
fn deal_is_slashed() {
    struct Case {
        name: &'static str,
        deal_start: ChainEpoch,
        deal_end: ChainEpoch,
        activation_epoch: ChainEpoch,
        termination_epoch: ChainEpoch,
        payment: TokenAmount,
    }

    let cases = [
        Case {
            name: "deal is slashed after the startepoch and then the first crontick happens",
            deal_start: 10,
            deal_end: 10 + 200 * EPOCHS_IN_DAY,
            activation_epoch: 5,
            termination_epoch: 15,
            payment: TokenAmount::from_atto(50), // (15 - 10) * 10 as deal storage fee is 10 per epoch
        },
        Case {
            name: "deal is slashed at the startepoch and then the first crontick happens",
            deal_start: 10,
            deal_end: 10 + 200 * EPOCHS_IN_DAY,
            activation_epoch: 5,
            termination_epoch: 10,
            payment: TokenAmount::zero(), // (10 - 10) * 10
        },
        Case {
            name: "deal is slashed before the startepoch and then the first crontick happens",
            deal_start: 10,
            deal_end: 10 + 200 * EPOCHS_IN_DAY,
            activation_epoch: 5,
            termination_epoch: 6,
            payment: TokenAmount::zero(), // (10 - 10) * 10
        },
        Case {
            name: "deal is terminated at the activation epoch and then the first crontick happens",
            deal_start: 10,
            deal_end: 10 + 200 * EPOCHS_IN_DAY,
            activation_epoch: 5,
            termination_epoch: 5,
            payment: TokenAmount::zero(), // (10 - 10) * 10
        },
        Case {
            name: "deal is slashed just BEFORE the end epoch",
            deal_start: 10,
            deal_end: 10 + 200 * EPOCHS_IN_DAY,
            activation_epoch: 5,
            termination_epoch: 19,
            payment: TokenAmount::from_atto(90), // (19 - 10) * 10
        },
    ];
    for tc in cases {
        eprintln!("Running testcase: {}", tc.name);
        let mut rt = setup();

        // publish and activate
        rt.set_epoch(tc.activation_epoch);
        let deal_id = publish_and_activate_deal(
            &mut rt,
            CLIENT_ADDR,
            &MinerAddresses::default(),
            tc.deal_start,
            tc.deal_end,
            tc.activation_epoch,
            SECTOR_EXPIRY,
        );
        let deal_proposal = get_deal_proposal(&mut rt, deal_id);

        // terminate
        rt.set_epoch(tc.termination_epoch);
        terminate_deals(&mut rt, PROVIDER_ADDR, &[deal_id]);

        // cron tick
        let cron_tick_epoch = process_epoch(tc.deal_start, deal_id);
        rt.set_epoch(cron_tick_epoch);

        let (pay, slashed) = cron_tick_and_assert_balances(
            &mut rt,
            CLIENT_ADDR,
            PROVIDER_ADDR,
            cron_tick_epoch,
            deal_id,
        );
        assert_eq!(tc.payment, pay);
        assert_eq!(deal_proposal.provider_collateral, slashed);
        assert_deal_deleted(&mut rt, deal_id, deal_proposal);

        check_state(&rt);
    }
}

const START_EPOCH: ChainEpoch = 50;
const END_EPOCH: ChainEpoch = 50 + 200 * EPOCHS_IN_DAY;

#[test]
fn deal_is_slashed_at_the_end_epoch_should_not_be_slashed_and_should_be_considered_expired() {
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

    // set current epoch to deal end epoch and attempt to slash it -> should not be slashed
    // as deal is considered to be expired.

    rt.set_epoch(END_EPOCH);
    terminate_deals(&mut rt, PROVIDER_ADDR, &[deal_id]);

    // on the next cron tick, it will be processed as expired
    let current = END_EPOCH + 300;
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
fn deal_payment_and_slashing_correctly_processed_in_same_crontick() {
    // start epoch should equal first processing epoch for logic to work
    let start_epoch: ChainEpoch = Policy::default().deal_updates_interval;
    let mut rt = setup();
    let deal_id = publish_and_activate_deal(
        &mut rt,
        CLIENT_ADDR,
        &MinerAddresses::default(),
        start_epoch,
        END_EPOCH,
        0,
        SECTOR_EXPIRY,
    );
    let deal_proposal = get_deal_proposal(&mut rt, deal_id);

    // move the current epoch to startEpoch so next cron epoch will be start + Interval
    let current = process_epoch(start_epoch, deal_id);
    rt.set_epoch(current);
    let (pay, slashed) =
        cron_tick_and_assert_balances(&mut rt, CLIENT_ADDR, PROVIDER_ADDR, current, deal_id);
    assert!(pay.is_zero());
    assert!(slashed.is_zero());

    // set slash epoch of deal
    let slash_epoch = current + Policy::default().deal_updates_interval + 1;
    rt.set_epoch(slash_epoch);
    terminate_deals(&mut rt, PROVIDER_ADDR, &[deal_id]);

    let duration = slash_epoch - current;
    let current = current + Policy::default().deal_updates_interval + 2;
    rt.set_epoch(current);
    let (pay, slashed) =
        cron_tick_and_assert_balances(&mut rt, CLIENT_ADDR, PROVIDER_ADDR, current, deal_id);
    assert_eq!(duration * &deal_proposal.storage_price_per_epoch, pay);
    assert_eq!(deal_proposal.provider_collateral, slashed);

    // deal should be deleted as it should have expired
    assert_deal_deleted(&mut rt, deal_id, deal_proposal);
    check_state(&rt);
}

#[test]
fn slash_multiple_deals_in_the_same_epoch() {
    let mut rt = setup();

    // three deals for slashing
    let deal_id1 = publish_and_activate_deal(
        &mut rt,
        CLIENT_ADDR,
        &MinerAddresses::default(),
        START_EPOCH,
        END_EPOCH,
        0,
        SECTOR_EXPIRY,
    );
    let deal_proposal1 = get_deal_proposal(&mut rt, deal_id1);

    let deal_id2 = publish_and_activate_deal(
        &mut rt,
        CLIENT_ADDR,
        &MinerAddresses::default(),
        START_EPOCH,
        END_EPOCH + 1,
        0,
        SECTOR_EXPIRY,
    );
    let deal_proposal2 = get_deal_proposal(&mut rt, deal_id2);

    let deal_id3 = publish_and_activate_deal(
        &mut rt,
        CLIENT_ADDR,
        &MinerAddresses::default(),
        START_EPOCH,
        END_EPOCH + 2,
        0,
        SECTOR_EXPIRY,
    );
    let deal_proposal3 = get_deal_proposal(&mut rt, deal_id3);

    // set slash epoch of deal at 100 epochs past last process epoch
    rt.set_epoch(process_epoch(START_EPOCH, deal_id3) + 100);
    terminate_deals(&mut rt, PROVIDER_ADDR, &[deal_id1, deal_id2, deal_id3]);

    // process slashing of deals 200 epochs later
    rt.set_epoch(process_epoch(START_EPOCH, deal_id3) + 300);
    let total_slashed = &deal_proposal1.provider_collateral
        + &deal_proposal2.provider_collateral
        + &deal_proposal3.provider_collateral;
    rt.expect_send(
        BURNT_FUNDS_ACTOR_ADDR,
        METHOD_SEND,
        RawBytes::default(),
        total_slashed,
        RawBytes::default(),
        ExitCode::OK,
    );
    cron_tick(&mut rt);

    assert_deal_deleted(&mut rt, deal_id1, deal_proposal1);
    assert_deal_deleted(&mut rt, deal_id2, deal_proposal2);
    assert_deal_deleted(&mut rt, deal_id3, deal_proposal3);
    check_state(&rt);
}

#[test]
fn regular_payments_till_deal_is_slashed_and_then_slashing_is_processed() {
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

    // move the current epoch to the process epoch + 5 so payment is made
    let process_start = process_epoch(START_EPOCH, deal_id);
    let current = process_start + 5;
    rt.set_epoch(current);

    // assert payment
    let (pay, slashed) =
        cron_tick_and_assert_balances(&mut rt, CLIENT_ADDR, PROVIDER_ADDR, current, deal_id);
    assert_eq!(pay, (5 + process_start - START_EPOCH) * &deal_proposal.storage_price_per_epoch);
    assert!(slashed.is_zero());

    // Setting the current epoch to before the next schedule will NOT make any changes as the deal
    // is still not scheduled
    let current = current + Policy::default().deal_updates_interval - 1;
    rt.set_epoch(current);
    cron_tick_no_change(&mut rt, CLIENT_ADDR, PROVIDER_ADDR);

    // a second cron tick for the same epoch should not change anything
    cron_tick_no_change(&mut rt, CLIENT_ADDR, PROVIDER_ADDR);

    // make another payment
    let current = current + 1;
    rt.set_epoch(current);
    let duration = Policy::default().deal_updates_interval;
    let (pay, slashed) =
        cron_tick_and_assert_balances(&mut rt, CLIENT_ADDR, PROVIDER_ADDR, current, deal_id);
    assert_eq!(pay, duration * &deal_proposal.storage_price_per_epoch);
    assert!(slashed.is_zero());

    // a second cron tick for the same epoch should not change anything
    cron_tick_no_change(&mut rt, CLIENT_ADDR, PROVIDER_ADDR);

    // now terminate the deal
    let slash_epoch = current + 1;
    rt.set_epoch(slash_epoch);
    let duration = slash_epoch - current;
    terminate_deals(&mut rt, PROVIDER_ADDR, &[deal_id]);

    // Setting the epoch to anything less than next schedule will not make any change even though the deal is slashed
    let current = current + Policy::default().deal_updates_interval - 1;
    rt.set_epoch(current);
    cron_tick_no_change(&mut rt, CLIENT_ADDR, PROVIDER_ADDR);

    // next epoch for cron schedule  -> payment will be made and deal will be slashed
    let current = current + 1;
    rt.set_epoch(current);
    let (pay, slashed) =
        cron_tick_and_assert_balances(&mut rt, CLIENT_ADDR, PROVIDER_ADDR, current, deal_id);
    assert_eq!(pay, duration * &deal_proposal.storage_price_per_epoch);
    assert_eq!(slashed, deal_proposal.provider_collateral);

    // deal should be deleted as it should have expired
    assert_deal_deleted(&mut rt, deal_id, deal_proposal);
    check_state(&rt);
}

#[test]
fn regular_payments_till_deal_expires_and_then_we_attempt_to_slash_it_but_it_will_not_be_slashed() {
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

    // move the current epoch to processEpoch + 5 so payment is made and assert payment
    let process_start = process_epoch(START_EPOCH, deal_id);
    let current = process_start + 5;
    rt.set_epoch(current);
    let (pay, slashed) =
        cron_tick_and_assert_balances(&mut rt, CLIENT_ADDR, PROVIDER_ADDR, current, deal_id);
    assert_eq!(pay, (5 + process_start - START_EPOCH) * &deal_proposal.storage_price_per_epoch);
    assert!(slashed.is_zero());

    //  Incrementing the current epoch another update interval will make another payment
    let current = current + Policy::default().deal_updates_interval;
    rt.set_epoch(current);
    let duration = Policy::default().deal_updates_interval;
    let (pay, slashed) =
        cron_tick_and_assert_balances(&mut rt, CLIENT_ADDR, PROVIDER_ADDR, current, deal_id);
    assert_eq!(pay, duration * &deal_proposal.storage_price_per_epoch);
    assert!(slashed.is_zero());

    // set current epoch to deal end epoch and attempt to slash it -> should not be slashed
    // as deal is considered to be expired.
    let duration = END_EPOCH - current;
    rt.set_epoch(END_EPOCH);
    terminate_deals(&mut rt, PROVIDER_ADDR, &[deal_id]);

    // next epoch for cron schedule is endEpoch + 300 ->
    // setting epoch to higher than that will cause deal to be expired, payment will be made
    // and deal will NOT be slashed
    let current = END_EPOCH + 300;
    rt.set_epoch(current);
    let (pay, slashed) =
        cron_tick_and_assert_balances(&mut rt, CLIENT_ADDR, PROVIDER_ADDR, current, deal_id);
    assert_eq!(pay, duration * &deal_proposal.storage_price_per_epoch);
    assert!(slashed.is_zero());

    // deal should be deleted as it should have expired
    assert_deal_deleted(&mut rt, deal_id, deal_proposal);
    check_state(&rt);
}
