// Copyright 2019-2022 ChainSafe Systems
// SPDX-License-Identifier: Apache-2.0, MIT
//! TODO: remove tests for legacy behaviour: https://github.com/filecoin-project/builtin-actors/issues/1389
use fil_actor_market::{rt_deal_cid, State};
use fil_actors_runtime::network::EPOCHS_IN_DAY;
use fil_actors_runtime::runtime::Runtime;
use fil_actors_runtime::{parse_uint_key, u64_key, SetMultimap, BURNT_FUNDS_ACTOR_ADDR};
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
    let rt = setup();

    let deal_id = generate_and_publish_deal(
        &rt,
        CLIENT_ADDR,
        &MinerAddresses::default(),
        START_EPOCH,
        END_EPOCH,
    );
    let deal_proposal = get_deal_proposal(&rt, deal_id);
    let dcid = rt_deal_cid(&rt, &deal_proposal).unwrap();

    // activate the deal
    rt.set_epoch(START_EPOCH - 1);
    activate_deals(&rt, SECTOR_EXPIRY, PROVIDER_ADDR, deal_proposal.start_epoch - 1, &[deal_id]);

    // cron tick at deal start epoch does not do anything
    rt.set_epoch(START_EPOCH);
    cron_tick_no_change(&rt, CLIENT_ADDR, PROVIDER_ADDR);

    let state: State = rt.get_state();
    // check pending deal proposal exists
    assert!(state.has_pending_deal(rt.store(), dcid).unwrap());

    // first cron tick at process epoch will clear the pending state and not reschedule the deal
    let deal_epoch = process_epoch(START_EPOCH, deal_id);
    rt.set_epoch(deal_epoch);
    cron_tick(&rt);

    // check that deal was not rescheduled
    let state: State = rt.get_state();
    let deal_ops = SetMultimap::from_root(rt.store(), &state.deal_ops_by_epoch).unwrap();

    // get into internals just to iterate through full data structure
    deal_ops
        .0
        .for_each(|key, _| {
            let epoch = parse_uint_key(key)? as i64;
            let epoch_ops = deal_ops.get(epoch).unwrap().unwrap();
            assert!(!epoch_ops.has(&u64_key(deal_id))?);
            Ok(())
        })
        .unwrap();

    assert!(!state.has_pending_deal(rt.store(), dcid).unwrap());

    check_state(&rt);
}

#[test]
fn deal_is_processed_after_its_end_epoch_should_expire_correctly() {
    let rt = setup();

    let activation_epoch = START_EPOCH - 1;
    rt.set_epoch(activation_epoch);
    let (deal_id, deal_proposal) = publish_and_activate_deal_legacy(
        &rt,
        CLIENT_ADDR,
        &MinerAddresses::default(),
        START_EPOCH,
        END_EPOCH,
        activation_epoch,
        SECTOR_EXPIRY,
    );

    rt.set_epoch(END_EPOCH + 100);
    let (pay, slashed) =
        cron_tick_and_assert_balances(&rt, CLIENT_ADDR, PROVIDER_ADDR, END_EPOCH + 100, deal_id);
    assert!(slashed.is_zero());
    let duration = END_EPOCH - START_EPOCH;
    assert_eq!(duration * &deal_proposal.storage_price_per_epoch, pay);
    assert_deal_deleted(&rt, deal_id, &deal_proposal);
    check_state(&rt);
}

#[test]
fn activation_after_deal_start_epoch_but_before_it_is_processed_fails() {
    let rt = setup();
    let deal_id = generate_and_publish_deal(
        &rt,
        CLIENT_ADDR,
        &MinerAddresses::default(),
        START_EPOCH,
        END_EPOCH,
    );

    // activate the deal after the start epoch
    let curr_epoch = START_EPOCH + 1;
    rt.set_epoch(curr_epoch);

    let res = activate_deals(&rt, SECTOR_EXPIRY, PROVIDER_ADDR, curr_epoch, &[deal_id]);
    assert_eq!(res.activation_results.codes(), vec![ExitCode::USR_ILLEGAL_ARGUMENT]);
    check_state(&rt);
}

#[test]
fn cron_processing_of_deal_after_missed_activation_should_fail_and_slash() {
    let rt = setup();
    let deal_id = generate_and_publish_deal(
        &rt,
        CLIENT_ADDR,
        &MinerAddresses::default(),
        START_EPOCH,
        END_EPOCH,
    );
    let deal_proposal = get_deal_proposal(&rt, deal_id);

    rt.set_epoch(process_epoch(START_EPOCH, deal_id));

    // FIXME: cron_tick calls 'VERIFIED_REGISTRY_ACTOR_ADDR' with the 'USE_BYTES_METHOD' method.
    rt.expect_send_simple(
        BURNT_FUNDS_ACTOR_ADDR,
        METHOD_SEND,
        None,
        deal_proposal.provider_collateral.clone(),
        None,
        ExitCode::OK,
    );
    cron_tick(&rt);

    assert_deal_deleted(&rt, deal_id, &deal_proposal);
    check_state(&rt);
}
