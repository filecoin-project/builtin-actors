// Copyright 2019-2022 ChainSafe Systems
// SPDX-License-Identifier: Apache-2.0, MIT

//! TODO: Revisit tests here and cleanup https://github.com/filecoin-project/builtin-actors/issues/1389
use fil_actors_runtime::network::EPOCHS_IN_DAY;
use fil_actors_runtime::runtime::Policy;
use fvm_shared::clock::ChainEpoch;
use fvm_shared::econ::TokenAmount;
use fvm_shared::sector::SectorNumber;
use num_traits::Zero;

mod harness;

use harness::*;

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
    for (i, tc) in cases.iter().enumerate() {
        eprintln!("Running testcase: {}", tc.name);
        let rt = setup();

        // publish and activate
        rt.set_epoch(tc.activation_epoch);
        let sector_number: SectorNumber = i as SectorNumber;
        let (deal_id, deal_proposal) = publish_and_activate_deal_legacy(
            &rt,
            CLIENT_ADDR,
            &MinerAddresses::default(),
            sector_number,
            tc.deal_start,
            tc.deal_end,
            tc.activation_epoch,
            tc.deal_end,
        );

        // terminate
        rt.set_epoch(tc.termination_epoch);
        let (pay, slashed) = terminate_deals_and_assert_balances(
            &rt,
            CLIENT_ADDR,
            PROVIDER_ADDR,
            &[sector_number],
            &[deal_id],
        );

        assert_eq!(tc.payment, pay);
        assert_eq!(deal_proposal.provider_collateral, slashed);

        // cron tick to remove final deal op state
        let cron_tick_epoch = process_epoch(tc.deal_start, deal_id);
        rt.set_epoch(cron_tick_epoch);
        cron_tick(&rt);

        assert_deal_deleted(&rt, deal_id, &deal_proposal, sector_number, true);

        check_state(&rt);
    }
}

const START_EPOCH: ChainEpoch = 50;
const DEAL_DURATION_EPOCHS: ChainEpoch = 200 * EPOCHS_IN_DAY;
const END_EPOCH: ChainEpoch = START_EPOCH + DEAL_DURATION_EPOCHS;
const SECTOR_NUMBER: SectorNumber = 7;
const SECTOR_EXPIRY: ChainEpoch = END_EPOCH + EPOCHS_IN_DAY;

#[test]
fn deal_is_slashed_at_the_end_epoch_should_not_be_slashed_and_should_be_considered_expired() {
    let rt = setup();
    let (deal_id, deal_proposal) = publish_and_activate_deal_legacy(
        &rt,
        CLIENT_ADDR,
        &MinerAddresses::default(),
        SECTOR_NUMBER,
        START_EPOCH,
        END_EPOCH,
        0,
        SECTOR_EXPIRY,
    );

    // set current epoch to deal end epoch and attempt to slash it -> should not be slashed
    // as deal is considered to be expired.
    rt.set_epoch(END_EPOCH);
    terminate_deals_and_assert_balances(&rt, CLIENT_ADDR, PROVIDER_ADDR, &[SECTOR_NUMBER], &[]);
    let duration = END_EPOCH - START_EPOCH;

    let current = END_EPOCH + 300;
    rt.set_epoch(current);
    let (pay, slashed) =
        cron_tick_and_assert_balances(&rt, CLIENT_ADDR, PROVIDER_ADDR, current, deal_id);
    assert_eq!(duration * &deal_proposal.storage_price_per_epoch, pay);
    assert!(slashed.is_zero());

    // deal should be deleted as it should have expired
    assert_deal_deleted(&rt, deal_id, &deal_proposal, SECTOR_NUMBER, true);

    check_state(&rt);
}

#[test]
fn deal_payment_and_slashing_correctly_processed_in_same_crontick() {
    // start epoch should equal first processing epoch for logic to work
    let start_epoch: ChainEpoch = Policy::default().deal_updates_interval;
    let end_epoch = start_epoch + DEAL_DURATION_EPOCHS;
    let sector_number = 7;
    let rt = setup();
    let (deal_id, deal_proposal) = publish_and_activate_deal_legacy(
        &rt,
        CLIENT_ADDR,
        &MinerAddresses::default(),
        sector_number,
        start_epoch,
        end_epoch,
        0,
        end_epoch,
    );

    // move the current epoch to startEpoch so next cron epoch will be start + Interval
    let current = process_epoch(start_epoch, deal_id);
    rt.set_epoch(current);
    let (pay, slashed) =
        cron_tick_and_assert_balances(&rt, CLIENT_ADDR, PROVIDER_ADDR, current, deal_id);
    assert!(pay.is_zero());
    assert!(slashed.is_zero());

    // set slash epoch of deal
    let slash_epoch = current + Policy::default().deal_updates_interval + 1;
    rt.set_epoch(slash_epoch);
    let (pay, slashed) = terminate_deals_and_assert_balances(
        &rt,
        CLIENT_ADDR,
        PROVIDER_ADDR,
        &[sector_number],
        &[deal_id],
    );
    let duration = slash_epoch - current;
    assert_eq!(duration * &deal_proposal.storage_price_per_epoch, pay);
    assert_eq!(deal_proposal.provider_collateral, slashed);

    let current = current + Policy::default().deal_updates_interval + 2;
    rt.set_epoch(current);
    cron_tick(&rt);

    // deal should be deleted as it should have expired
    assert_deal_deleted(&rt, deal_id, &deal_proposal, sector_number, true);
    check_state(&rt);
}

#[test]
fn slash_multiple_deals_in_the_same_epoch() {
    let rt = setup();

    // three deals for slashing
    let (deal_id1, deal_proposal1) = publish_and_activate_deal_legacy(
        &rt,
        CLIENT_ADDR,
        &MinerAddresses::default(),
        SECTOR_NUMBER,
        START_EPOCH,
        END_EPOCH,
        0,
        SECTOR_EXPIRY,
    );

    let (deal_id2, deal_proposal2) = publish_and_activate_deal_legacy(
        &rt,
        CLIENT_ADDR,
        &MinerAddresses::default(),
        SECTOR_NUMBER,
        START_EPOCH,
        END_EPOCH + 1,
        0,
        SECTOR_EXPIRY,
    );

    let (deal_id3, deal_proposal3) = publish_and_activate_deal_legacy(
        &rt,
        CLIENT_ADDR,
        &MinerAddresses::default(),
        SECTOR_NUMBER,
        START_EPOCH,
        END_EPOCH + 2,
        0,
        SECTOR_EXPIRY,
    );

    // set slash epoch of deal at 100 epochs past last process epoch
    let epoch = process_epoch(START_EPOCH, deal_id3) + 100;
    rt.set_epoch(process_epoch(START_EPOCH, deal_id3) + 100);
    terminate_deals_and_assert_balances(
        &rt,
        CLIENT_ADDR,
        PROVIDER_ADDR,
        &[SECTOR_NUMBER],
        &[deal_id1, deal_id2, deal_id3],
    );

    // next epoch run should clean up any remaining state
    rt.set_epoch(epoch + 1);
    cron_tick(&rt);

    assert_deal_deleted(&rt, deal_id1, &deal_proposal1, SECTOR_NUMBER, true);
    assert_deal_deleted(&rt, deal_id2, &deal_proposal2, SECTOR_NUMBER, true);
    assert_deal_deleted(&rt, deal_id3, &deal_proposal3, SECTOR_NUMBER, true);
    check_state(&rt);
}

#[test]
fn regular_payments_till_deal_is_slashed_and_then_slashing_is_processed() {
    let rt = setup();
    let (deal_id, deal_proposal) = publish_and_activate_deal_legacy(
        &rt,
        CLIENT_ADDR,
        &MinerAddresses::default(),
        SECTOR_NUMBER,
        START_EPOCH,
        END_EPOCH,
        0,
        SECTOR_EXPIRY,
    );

    // move the current epoch to the process epoch + 5 so payment is made
    let process_start = process_epoch(START_EPOCH, deal_id);
    let current = rt.set_epoch(process_start + 5);

    // assert payment
    let (pay, slashed) =
        cron_tick_and_assert_balances(&rt, CLIENT_ADDR, PROVIDER_ADDR, current, deal_id);
    assert_eq!(pay, (5 + process_start - START_EPOCH) * &deal_proposal.storage_price_per_epoch);
    assert!(slashed.is_zero());

    // Setting the current epoch to before the next schedule will NOT make any changes as the deal
    // is still not scheduled
    rt.set_epoch(process_start + Policy::default().deal_updates_interval - 1);
    cron_tick_no_change(&rt, CLIENT_ADDR, PROVIDER_ADDR);

    // a second cron tick for the same epoch should not change anything
    cron_tick_no_change(&rt, CLIENT_ADDR, PROVIDER_ADDR);

    // make another payment
    let current = rt.set_epoch(process_start + Policy::default().deal_updates_interval);
    let duration = Policy::default().deal_updates_interval - 5;
    let (pay, slashed) =
        cron_tick_and_assert_balances(&rt, CLIENT_ADDR, PROVIDER_ADDR, current, deal_id);
    assert_eq!(pay, duration * &deal_proposal.storage_price_per_epoch);
    assert!(slashed.is_zero());

    // a second cron tick for the same epoch should not change anything
    cron_tick_no_change(&rt, CLIENT_ADDR, PROVIDER_ADDR);

    // now terminate the deal 1 epoch later
    rt.set_epoch(process_start + Policy::default().deal_updates_interval + 1);
    let (pay, slashed) = terminate_deals_and_assert_balances(
        &rt,
        CLIENT_ADDR,
        PROVIDER_ADDR,
        &[SECTOR_NUMBER],
        &[deal_id],
    );
    assert_eq!(pay, 1 * &deal_proposal.storage_price_per_epoch);
    assert_eq!(slashed, deal_proposal.provider_collateral);

    // Setting the epoch to anything less than next schedule will not make any change even though the deal is slashed
    rt.set_epoch(process_start + 2 * Policy::default().deal_updates_interval - 1);
    cron_tick_no_change(&rt, CLIENT_ADDR, PROVIDER_ADDR);

    // next epoch for cron schedule  -> payment will be made and deal will be slashed
    rt.set_epoch(process_start + 2 * Policy::default().deal_updates_interval);
    cron_tick(&rt);

    // deal should be deleted as it should have expired
    assert_deal_deleted(&rt, deal_id, &deal_proposal, SECTOR_NUMBER, true);
    check_state(&rt);
}

#[test]
fn regular_payments_till_deal_expires_and_then_we_attempt_to_slash_it_but_it_will_not_be_slashed() {
    let rt = setup();
    let (deal_id, deal_proposal) = publish_and_activate_deal_legacy(
        &rt,
        CLIENT_ADDR,
        &MinerAddresses::default(),
        SECTOR_NUMBER,
        START_EPOCH,
        END_EPOCH,
        0,
        SECTOR_EXPIRY,
    );

    // move the current epoch to processEpoch + 5 so payment is made and assert payment
    let process_start = process_epoch(START_EPOCH, deal_id);
    let current = process_start + 5;
    rt.set_epoch(current);
    let (pay, slashed) =
        cron_tick_and_assert_balances(&rt, CLIENT_ADDR, PROVIDER_ADDR, current, deal_id);
    assert_eq!(pay, (5 + process_start - START_EPOCH) * &deal_proposal.storage_price_per_epoch);
    assert!(slashed.is_zero());

    //  Incrementing the current epoch another update interval will make another payment
    let current = current + Policy::default().deal_updates_interval;
    rt.set_epoch(current);
    let duration = Policy::default().deal_updates_interval;
    let (pay, slashed) =
        cron_tick_and_assert_balances(&rt, CLIENT_ADDR, PROVIDER_ADDR, current, deal_id);
    assert_eq!(pay, duration * &deal_proposal.storage_price_per_epoch);
    assert!(slashed.is_zero());

    // set current epoch to deal end epoch and attempt to slash it -> should not be slashed
    // as deal is considered to be expired.
    let duration = END_EPOCH - current;
    rt.set_epoch(END_EPOCH);
    terminate_deals(&rt, PROVIDER_ADDR, &[SECTOR_NUMBER], &[]);

    // next epoch for cron schedule is endEpoch + 300 ->
    // setting epoch to higher than that will cause deal to be expired, payment will be made
    // and deal will NOT be slashed
    let current = END_EPOCH + 300;
    rt.set_epoch(current);
    let (pay, slashed) =
        cron_tick_and_assert_balances(&rt, CLIENT_ADDR, PROVIDER_ADDR, current, deal_id);
    assert_eq!(pay, duration * &deal_proposal.storage_price_per_epoch);
    assert!(slashed.is_zero());

    // deal should be deleted as it should have expired
    assert_deal_deleted(&rt, deal_id, &deal_proposal, SECTOR_NUMBER, true);
    check_state(&rt);
}
