use fil_actors_runtime::network::EPOCHS_IN_DAY;
use fil_actors_runtime::BURNT_FUNDS_ACTOR_ADDR;
use fvm_shared::clock::ChainEpoch;
use fvm_shared::error::ExitCode;
use fvm_shared::METHOD_SEND;

mod harness;

use harness::*;
use regex::Regex;

const START_EPOCH: ChainEpoch = 50;
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

    // do a cron tick for it -> should time out and get slashed
    rt.set_epoch(process_epoch(START_EPOCH, deal_id));
    rt.expect_send_simple(
        BURNT_FUNDS_ACTOR_ADDR,
        METHOD_SEND,
        None,
        deal_proposal.provider_collateral.clone(),
        None,
        ExitCode::OK,
    );

    process_deal_updates(&rt, CLIENT_ADDR, vec![deal_id]);

    let client_acct = get_balance(&rt, &CLIENT_ADDR);
    assert_eq!(c_escrow, client_acct.balance);
    assert!(client_acct.locked.is_zero());
    assert_account_zero(&rt, PROVIDER_ADDR);
    assert_deal_deleted(&rt, deal_id, deal_proposal);

    check_state_with_expected(
        &rt,
        &[Regex::new(&format!(
            "^deal op found for deal id {deal_id} with missing proposal at epoch \\d+$"
        ))
        .unwrap()],
    );

    // cron tick should remove the dangling deal op from the queue
    cron_tick(&rt);

    check_state(&rt);
}
