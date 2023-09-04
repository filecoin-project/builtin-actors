use fil_actors_runtime::network::EPOCHS_IN_DAY;
use fil_actors_runtime::BURNT_FUNDS_ACTOR_ADDR;
use fvm_shared::clock::ChainEpoch;
use fvm_shared::error::ExitCode;
use fvm_shared::METHOD_SEND;

mod harness;

use harness::*;

const START_EPOCH: ChainEpoch = 0;
const END_EPOCH: ChainEpoch = START_EPOCH + 200 * EPOCHS_IN_DAY;

#[test]
fn timedout_deal_is_slashed_and_deleted() {
    let rt = setup();
    let deal_id = generate_and_publish_deal(
        &rt,
        CLIENT_ADDR,
        &MinerAddresses::default(),
        START_EPOCH,
        END_EPOCH,
    );
    let deal_proposal = get_deal_proposal(&rt, deal_id);

    let c_escrow = get_balance(&rt, &CLIENT_ADDR).balance;

    // advance to start_epoch without activating
    rt.set_epoch(process_epoch(START_EPOCH, deal_id));
    rt.expect_send_simple(
        BURNT_FUNDS_ACTOR_ADDR,
        METHOD_SEND,
        None,
        deal_proposal.provider_collateral.clone(),
        None,
        ExitCode::OK,
    );

    // settle deal payments -> should time out and get slashed
    settle_deal_payments(&rt, CLIENT_ADDR, vec![deal_id]);

    let client_acct = get_balance(&rt, &CLIENT_ADDR);
    assert_eq!(c_escrow, client_acct.balance);
    assert!(client_acct.locked.is_zero());
    assert_account_zero(&rt, PROVIDER_ADDR);
    assert_deal_deleted(&rt, deal_id, deal_proposal);

    check_state(&rt);

    // cron tick should remove the dangling deal op from the queue
    cron_tick(&rt);
    assert_deal_ops_clean(&rt);
}

// TODO: Revisit and cleanup https://github.com/filecoin-project/builtin-actors/issues/1389
#[test]
fn can_manually_settle_deals_in_the_cron_queue() {
    let rt = setup();
    let addrs = MinerAddresses::default();
    // create a legacy deal that is managed by cron
    let deal_id = publish_and_activate_deal_legacy(
        &rt,
        CLIENT_ADDR,
        &addrs,
        START_EPOCH,
        END_EPOCH,
        0,
        END_EPOCH,
    );
    let deal_proposal = get_deal_proposal(&rt, deal_id);

    let client_before = get_balance(&rt, &CLIENT_ADDR);
    let provider_before = get_balance(&rt, &addrs.provider);

    // advance to some epoch while the deal is active
    rt.set_epoch(START_EPOCH + 100);

    // manually call settle_deal_payments
    let ret = settle_deal_payments(&rt, addrs.provider, vec![deal_id]);
    let payment = ret.settlements[0].payment.clone();
    assert_eq!(&payment, &(&deal_proposal.storage_price_per_epoch * 100));

    // assert incremental payment was performed correctly
    let incremental_client_escrow = &client_before.balance - &payment;
    let incremental_provider_escrow = &provider_before.balance + &payment;
    let client_updated = get_balance(&rt, &CLIENT_ADDR);
    let provider_updated = get_balance(&rt, &addrs.provider);
    assert_eq!(&client_updated.balance, &incremental_client_escrow);
    assert_eq!(&provider_updated.balance, &incremental_provider_escrow);

    // advance to deal end epoch and call cron
    rt.set_epoch(END_EPOCH);
    cron_tick(&rt);

    // payments were calculated correctly, accounting for incremental payment already made
    let total_duration = END_EPOCH - START_EPOCH;
    let total_payment = &deal_proposal.storage_price_per_epoch * total_duration;
    let final_client_escrow = &client_before.balance - &total_payment;
    let final_provider_escrow = &provider_before.balance + &total_payment;
    let client_after = get_balance(&rt, &CLIENT_ADDR);
    let provider_after = get_balance(&rt, &addrs.provider);
    assert_eq!(&client_after.balance, &final_client_escrow);
    assert_eq!(&provider_after.balance, &final_provider_escrow);

    // cleaned up by cron
    assert_deal_deleted(&rt, deal_id, deal_proposal)
}
