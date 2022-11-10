// Copyright 2019-2022 ChainSafe Systems
// SPDX-License-Identifier: Apache-2.0, MIT

use fil_actors_runtime_common::network::EPOCHS_IN_DAY;
use fil_actors_runtime_common::runtime::Policy;
use fil_actors_runtime_common::test_utils::*;
use fil_actors_runtime_common::BURNT_FUNDS_ACTOR_ADDR;
use fvm_ipld_encoding::RawBytes;
use fvm_shared::clock::ChainEpoch;
use fvm_shared::error::ExitCode;
use fvm_shared::METHOD_SEND;

mod harness;
use harness::*;

const START_EPOCH: ChainEpoch = 50;
const END_EPOCH: ChainEpoch = START_EPOCH + 200 * EPOCHS_IN_DAY;
const SECTOR_EXPIRY: ChainEpoch = END_EPOCH + 1;

#[test]
fn cron_processing_happens_at_processing_epoch_not_start_epoch() {
    let mut rt = setup();

    let deal_id = generate_and_publish_deal(
        &mut rt,
        CLIENT_ADDR,
        &MinerAddresses::default(),
        START_EPOCH,
        END_EPOCH,
    );
    let deal_proposal = get_deal_proposal(&mut rt, deal_id);

    // activate the deal
    rt.set_epoch(START_EPOCH - 1);
    activate_deals(
        &mut rt,
        SECTOR_EXPIRY,
        PROVIDER_ADDR,
        deal_proposal.start_epoch - 1,
        &[deal_id],
    );

    // cron tick at deal start epoch does not do anything
    rt.set_epoch(START_EPOCH);
    cron_tick_no_change(&mut rt, CLIENT_ADDR, PROVIDER_ADDR);

    // first cron tick at process epoch will make payment and schedule the deal for next epoch
    let deal_epoch = process_epoch(START_EPOCH, deal_id);
    rt.set_epoch(deal_epoch);
    let (pay, _) =
        cron_tick_and_assert_balances(&mut rt, CLIENT_ADDR, PROVIDER_ADDR, deal_epoch, deal_id);
    let duration = deal_epoch - START_EPOCH;
    assert_eq!(duration * &deal_proposal.storage_price_per_epoch, pay);

    // payment at next epoch
    let new_epoch = deal_epoch + Policy::default().deal_updates_interval;
    rt.set_epoch(new_epoch);
    let (pay, _) =
        cron_tick_and_assert_balances(&mut rt, CLIENT_ADDR, PROVIDER_ADDR, new_epoch, deal_id);
    let duration = new_epoch - deal_epoch;
    assert_eq!(duration * &deal_proposal.storage_price_per_epoch, pay);

    check_state(&rt);
}

#[test]
fn deals_are_scheduled_for_expiry_later_than_the_end_epoch() {
    let mut rt = setup();
    let deal_id = generate_and_publish_deal(
        &mut rt,
        CLIENT_ADDR,
        &MinerAddresses::default(),
        START_EPOCH,
        END_EPOCH,
    );
    let deal_proposal = get_deal_proposal(&mut rt, deal_id);

    rt.set_epoch(START_EPOCH - 1);
    activate_deals(
        &mut rt,
        SECTOR_EXPIRY,
        PROVIDER_ADDR,
        deal_proposal.start_epoch - 1,
        &[deal_id],
    );

    // a cron tick at end epoch -1 schedules the deal for later than end epoch
    let curr = END_EPOCH - 1;
    rt.set_epoch(curr);
    let duration = curr - START_EPOCH;
    let (pay, _) =
        cron_tick_and_assert_balances(&mut rt, CLIENT_ADDR, PROVIDER_ADDR, curr, deal_id);
    assert_eq!(duration * &deal_proposal.storage_price_per_epoch, pay);

    // cron tick at end epoch does NOT expire the deal
    rt.set_epoch(END_EPOCH);
    cron_tick_no_change(&mut rt, CLIENT_ADDR, PROVIDER_ADDR);
    let _found = get_deal_proposal(&mut rt, deal_id);

    // cron tick at nextEpoch expires the deal -> payment is ONLY for one epoch
    let curr = curr + Policy::default().deal_updates_interval;
    rt.set_epoch(curr);
    let (pay, _) =
        cron_tick_and_assert_balances(&mut rt, CLIENT_ADDR, PROVIDER_ADDR, curr, deal_id);
    assert_eq!(&deal_proposal.storage_price_per_epoch, &pay);
    assert_deal_deleted(&mut rt, deal_id, deal_proposal);
    check_state(&rt);
}

#[test]
fn deal_is_processed_after_its_end_epoch_should_expire_correctly() {
    let mut rt = setup();

    let activation_epoch = START_EPOCH - 1;
    rt.set_epoch(activation_epoch);
    let deal_id = publish_and_activate_deal(
        &mut rt,
        CLIENT_ADDR,
        &MinerAddresses::default(),
        START_EPOCH,
        END_EPOCH,
        activation_epoch,
        SECTOR_EXPIRY,
    );
    let deal_proposal = get_deal_proposal(&mut rt, deal_id);

    rt.set_epoch(END_EPOCH + 100);
    let (pay, slashed) = cron_tick_and_assert_balances(
        &mut rt,
        CLIENT_ADDR,
        PROVIDER_ADDR,
        END_EPOCH + 100,
        deal_id,
    );
    assert!(slashed.is_zero());
    let duration = END_EPOCH - START_EPOCH;
    assert_eq!(duration * &deal_proposal.storage_price_per_epoch, pay);
    assert_deal_deleted(&mut rt, deal_id, deal_proposal);
    check_state(&rt);
}

#[test]
fn activation_after_deal_start_epoch_but_before_it_is_processed_fails() {
    let mut rt = setup();
    let deal_id = generate_and_publish_deal(
        &mut rt,
        CLIENT_ADDR,
        &MinerAddresses::default(),
        START_EPOCH,
        END_EPOCH,
    );

    // activate the deal after the start epoch
    let curr_epoch = START_EPOCH + 1;
    rt.set_epoch(curr_epoch);

    expect_abort(
        ExitCode::USR_ILLEGAL_ARGUMENT,
        activate_deals_raw(&mut rt, SECTOR_EXPIRY, PROVIDER_ADDR, curr_epoch, &[deal_id]),
    );
    check_state(&rt);
}

#[test]
fn cron_processing_of_deal_after_missed_activation_should_fail_and_slash() {
    let mut rt = setup();
    let deal_id = generate_and_publish_deal(
        &mut rt,
        CLIENT_ADDR,
        &MinerAddresses::default(),
        START_EPOCH,
        END_EPOCH,
    );
    let deal_proposal = get_deal_proposal(&mut rt, deal_id);

    rt.set_epoch(process_epoch(START_EPOCH, deal_id));

    // FIXME: cron_tick calls 'VERIFIED_REGISTRY_ACTOR_ADDR' with the 'USE_BYTES_METHOD' method.
    rt.expect_send(
        BURNT_FUNDS_ACTOR_ADDR,
        METHOD_SEND,
        RawBytes::default(),
        deal_proposal.provider_collateral.clone(),
        RawBytes::default(),
        ExitCode::OK,
    );
    cron_tick(&mut rt);

    assert_deal_deleted(&mut rt, deal_id, deal_proposal);
    check_state(&rt);
}
