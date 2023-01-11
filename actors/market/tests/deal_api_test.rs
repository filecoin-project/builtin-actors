use fvm_ipld_encoding::ipld_block::IpldBlock;
use fvm_shared::clock::ChainEpoch;
use fvm_shared::error::ExitCode;
use fvm_shared::METHOD_SEND;
use serde::de::DeserializeOwned;

use fil_actor_market::{
    Actor as MarketActor, DealQueryParams, GetDealActivationReturn, GetDealClientCollateralReturn,
    GetDealClientReturn, GetDealDataCommitmentReturn, GetDealLabelReturn,
    GetDealProviderCollateralReturn, GetDealProviderReturn, GetDealTermReturn,
    GetDealTotalPriceReturn, GetDealVerifiedReturn, Method, EX_DEAL_EXPIRED,
};
use fil_actors_runtime::network::EPOCHS_IN_DAY;
use fil_actors_runtime::runtime::policy_constants::DEAL_UPDATES_INTERVAL;
use fil_actors_runtime::test_utils::{
    expect_abort_contains_message, MockRuntime, ACCOUNT_ACTOR_CODE_ID,
};
use fil_actors_runtime::ActorError;
use fil_actors_runtime::BURNT_FUNDS_ACTOR_ADDR;
use harness::*;

mod harness;

#[test]
fn proposal_data() {
    let start_epoch = 1000;
    let end_epoch = start_epoch + 200 * EPOCHS_IN_DAY;
    let publish_epoch = ChainEpoch::from(1);

    let mut rt = setup();
    rt.set_epoch(publish_epoch);
    let next_allocation_id = 1;

    let proposal = generate_deal_and_add_funds(
        &mut rt,
        CLIENT_ADDR,
        &MinerAddresses::default(),
        start_epoch,
        end_epoch,
    );
    rt.set_caller(*ACCOUNT_ACTOR_CODE_ID, WORKER_ADDR);
    let id =
        publish_deals(&mut rt, &MinerAddresses::default(), &[proposal.clone()], next_allocation_id)
            [0];

    let data: GetDealDataCommitmentReturn =
        query_deal(&mut rt, Method::GetDealDataCommitmentExported, id);
    assert_eq!(proposal.piece_cid, data.data);
    assert_eq!(proposal.piece_size, data.size);

    let client: GetDealClientReturn = query_deal(&mut rt, Method::GetDealClientExported, id);
    assert_eq!(proposal.client.id().unwrap(), client.client);

    let provider: GetDealProviderReturn = query_deal(&mut rt, Method::GetDealProviderExported, id);
    assert_eq!(proposal.provider.id().unwrap(), provider.provider);

    let label: GetDealLabelReturn = query_deal(&mut rt, Method::GetDealLabelExported, id);
    assert_eq!(proposal.label, label.label);

    let term: GetDealTermReturn = query_deal(&mut rt, Method::GetDealTermExported, id);
    assert_eq!(proposal.start_epoch, term.start);
    assert_eq!(proposal.duration(), term.duration);

    let price: GetDealTotalPriceReturn = query_deal(&mut rt, Method::GetDealTotalPriceExported, id);
    assert_eq!(proposal.total_storage_fee(), price.total_price);

    let client_collateral: GetDealClientCollateralReturn =
        query_deal(&mut rt, Method::GetDealClientCollateralExported, id);
    assert_eq!(proposal.client_collateral, client_collateral.collateral);

    let provider_collateral: GetDealProviderCollateralReturn =
        query_deal(&mut rt, Method::GetDealProviderCollateralExported, id);
    assert_eq!(proposal.provider_collateral, provider_collateral.collateral);

    let verified: GetDealVerifiedReturn = query_deal(&mut rt, Method::GetDealVerifiedExported, id);
    assert_eq!(proposal.verified_deal, verified.verified);

    check_state(&rt);
}

#[test]
fn activation() {
    let start_epoch = 10;
    let end_epoch = start_epoch + 180 * EPOCHS_IN_DAY;
    let publish_epoch = ChainEpoch::from(1);

    let mut rt = setup();
    rt.set_epoch(publish_epoch);
    let next_allocation_id = 1;

    let proposal = generate_deal_and_add_funds(
        &mut rt,
        CLIENT_ADDR,
        &MinerAddresses::default(),
        start_epoch,
        end_epoch,
    );
    rt.set_caller(*ACCOUNT_ACTOR_CODE_ID, WORKER_ADDR);
    let id =
        publish_deals(&mut rt, &MinerAddresses::default(), &[proposal.clone()], next_allocation_id)
            [0];

    let activation: GetDealActivationReturn =
        query_deal(&mut rt, Method::GetDealActivationExported, id);
    assert_eq!(-1, activation.activated);
    assert_eq!(-1, activation.terminated);

    // activate the deal
    let activate_epoch = start_epoch - 2;
    rt.set_epoch(activate_epoch);
    activate_deals(&mut rt, end_epoch + 1, PROVIDER_ADDR, activate_epoch, &[id]);
    let activation: GetDealActivationReturn =
        query_deal(&mut rt, Method::GetDealActivationExported, id);
    assert_eq!(activate_epoch, activation.activated);
    assert_eq!(-1, activation.terminated);

    // terminate early
    let terminate_epoch = activate_epoch + 100;
    rt.set_epoch(terminate_epoch);
    terminate_deals(&mut rt, PROVIDER_ADDR, &[id]);
    let activation: GetDealActivationReturn =
        query_deal(&mut rt, Method::GetDealActivationExported, id);
    assert_eq!(activate_epoch, activation.activated);
    assert_eq!(terminate_epoch, activation.terminated);

    // Clean up state
    let clean_epoch = terminate_epoch + DEAL_UPDATES_INTERVAL;
    rt.set_epoch(clean_epoch);
    rt.expect_send(
        BURNT_FUNDS_ACTOR_ADDR,
        METHOD_SEND,
        None,
        proposal.provider_collateral,
        None,
        ExitCode::OK,
    );
    cron_tick(&mut rt);
    expect_abort_contains_message(
        EX_DEAL_EXPIRED,
        "expired",
        query_deal_raw(&mut rt, Method::GetDealActivationExported, id),
    );

    // Non-existent deal is NOT FOUND
    expect_abort_contains_message(
        ExitCode::USR_NOT_FOUND,
        "no such deal",
        query_deal_raw(&mut rt, Method::GetDealActivationExported, id + 1),
    );
}

fn query_deal<T: DeserializeOwned>(rt: &mut MockRuntime, method: Method, id: u64) -> T {
    query_deal_raw(rt, method, id).unwrap().unwrap().deserialize().unwrap()
}

fn query_deal_raw(
    rt: &mut MockRuntime,
    method: Method,
    id: u64,
) -> Result<Option<IpldBlock>, ActorError> {
    let params = DealQueryParams { id };
    rt.expect_validate_caller_any();
    rt.call::<MarketActor>(method as u64, IpldBlock::serialize_cbor(&params).unwrap())
}
