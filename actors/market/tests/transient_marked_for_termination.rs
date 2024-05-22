//! TODO: can be removed after https://github.com/filecoin-project/builtin-actors/issues/1388 is resolved
//! in the meantime, this asserts the behaviour for the set of unprocessed deals already marked-for-termination after
//! the code is updated to perform synchronous termination.

mod harness;

use std::collections::BTreeMap;

use fil_actor_market::{
    Actor as MarketActor, DealQueryParams, DealSettlementSummary, Method, State, EX_DEAL_EXPIRED,
};
use fil_actors_runtime::{
    runtime::Runtime,
    test_utils::{expect_abort_contains_message, MockRuntime},
    ActorError, BURNT_FUNDS_ACTOR_ADDR, EPOCHS_IN_DAY,
};
use fvm_ipld_encoding::ipld_block::IpldBlock;
use fvm_shared::{clock::ChainEpoch, error::ExitCode};
use harness::*;

const START_EPOCH: ChainEpoch = 5;
const SYNCHRONOUS_TERMINATION_SWITCHOVER_EPOCH: ChainEpoch = 200;
const END_EPOCH: ChainEpoch = SYNCHRONOUS_TERMINATION_SWITCHOVER_EPOCH + 200 * EPOCHS_IN_DAY;

#[test]
fn deal_scheduled_for_termination_cannot_be_settled_manually() {
    let rt = setup();
    let sector_number = 7;

    let (deal_id_1, deal_1_prop) = publish_and_activate_deal_legacy(
        &rt,
        CLIENT_ADDR,
        &MinerAddresses::default(),
        sector_number,
        START_EPOCH,
        END_EPOCH,
        0,
        END_EPOCH,
    );

    // mark this deal for termination
    let (slashed_deal, slashed_prop) = publish_and_activate_deal_legacy(
        &rt,
        CLIENT_ADDR,
        &MinerAddresses::default(),
        sector_number,
        START_EPOCH,
        END_EPOCH,
        0,
        END_EPOCH,
    );

    let slashed_epoch = SYNCHRONOUS_TERMINATION_SWITCHOVER_EPOCH - 1;
    let scheduled_epoch = SYNCHRONOUS_TERMINATION_SWITCHOVER_EPOCH + 2;

    // simulate one of the deals being marked for termination before the code switchover but scheduled for cron after
    {
        let mut state = rt.get_state::<State>();

        // slashing before the code switchover just marks the epoch in DealState
        let mut slashed_deal_state =
            state.remove_deal_state(rt.store(), slashed_deal).unwrap().unwrap();
        slashed_deal_state.slash_epoch = slashed_epoch;
        state.put_deal_states(rt.store(), &[(slashed_deal, slashed_deal_state)]).unwrap();

        // actual slashing scheduled for cron after the code switchover
        let mut deals_by_epoch = BTreeMap::new();
        deals_by_epoch.insert(scheduled_epoch, vec![slashed_deal]);
        state.put_batch_deals_by_epoch(rt.store(), &deals_by_epoch).unwrap();

        rt.replace_state(&state);
    }

    // code updated before cron is run
    rt.set_epoch(SYNCHRONOUS_TERMINATION_SWITCHOVER_EPOCH);

    // attempt to settle payment for both deals - fails because one deal is marked-for-termination
    settle_deal_payments_expect_abort(
        &rt,
        PROVIDER_ADDR,
        &[deal_id_1, slashed_deal],
        ExitCode::USR_ILLEGAL_ARGUMENT,
    );

    // advance cron to scheduled time and terminate it via cron
    rt.set_epoch(scheduled_epoch);
    rt.expect_send_simple(
        BURNT_FUNDS_ACTOR_ADDR,
        0,
        None,
        slashed_prop.provider_collateral.clone(),
        None,
        ExitCode::OK,
    );
    expect_emitted(
        &rt,
        "deal-terminated",
        slashed_deal,
        slashed_prop.client.id().unwrap(),
        slashed_prop.provider.id().unwrap(),
    );
    cron_tick(&rt);

    // assert that the slashed deal was terminated
    assert_deal_deleted(&rt, slashed_deal, &slashed_prop, sector_number, false);

    // attempt to settle payment for both deals again - partially succeeds because not found deals are ignored
    rt.set_epoch(scheduled_epoch + 1);
    let ret = settle_deal_payments(&rt, PROVIDER_ADDR, &[deal_id_1, slashed_deal], &[], &[]);
    let expected_payment =
        deal_1_prop.storage_price_per_epoch * (scheduled_epoch + 1 - START_EPOCH);
    assert_eq!(ret.results.codes(), vec![ExitCode::OK, EX_DEAL_EXPIRED]);
    assert_eq!(
        ret.settlements[0],
        DealSettlementSummary { completed: false, payment: expected_payment }
    );
}

#[test]
fn cannot_get_deal_activation_for_marked_for_termination_deal() {
    let rt = setup();
    let sector_number = 7;
    // create a deal and mark it for termination
    let (slashed_deal, _) = publish_and_activate_deal_legacy(
        &rt,
        CLIENT_ADDR,
        &MinerAddresses::default(),
        sector_number,
        START_EPOCH,
        END_EPOCH,
        0,
        END_EPOCH,
    );

    let slashed_epoch = SYNCHRONOUS_TERMINATION_SWITCHOVER_EPOCH - 1;
    let scheduled_epoch = SYNCHRONOUS_TERMINATION_SWITCHOVER_EPOCH + 2;

    // simulate one of the deals being marked for termination before the code switchover but scheduled for cron after
    {
        let mut state = rt.get_state::<State>();

        // slashing before the code switchover just marks the epoch in DealState
        let mut slashed_deal_state =
            state.remove_deal_state(rt.store(), slashed_deal).unwrap().unwrap();
        slashed_deal_state.slash_epoch = slashed_epoch;
        state.put_deal_states(rt.store(), &[(slashed_deal, slashed_deal_state)]).unwrap();

        // actual slashing scheduled for cron after the code switchover
        let mut deals_by_epoch = BTreeMap::new();
        deals_by_epoch.insert(scheduled_epoch, vec![slashed_deal]);
        state.put_batch_deals_by_epoch(rt.store(), &deals_by_epoch).unwrap();

        rt.replace_state(&state);
    }

    // code updated before cron is run
    rt.set_epoch(SYNCHRONOUS_TERMINATION_SWITCHOVER_EPOCH);

    // attempt to get deal activation for the slashed deal - fails because it's marked-for-termination
    expect_abort_contains_message(
        EX_DEAL_EXPIRED,
        &format!("deal {slashed_deal} expired"),
        query_deal_raw(&rt, Method::GetDealActivationExported, slashed_deal),
    );
}

fn query_deal_raw(
    rt: &MockRuntime,
    method: Method,
    id: u64,
) -> Result<Option<IpldBlock>, ActorError> {
    let params = DealQueryParams { id };
    rt.expect_validate_caller_any();
    rt.call::<MarketActor>(method as u64, IpldBlock::serialize_cbor(&params).unwrap())
}
