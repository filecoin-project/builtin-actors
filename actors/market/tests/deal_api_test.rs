use fvm_ipld_encoding::ipld_block::IpldBlock;
use fvm_shared::clock::{ChainEpoch, EPOCH_UNDEFINED};
use fvm_shared::econ::TokenAmount;
use fvm_shared::error::ExitCode;
use num_traits::Zero;
use serde::de::DeserializeOwned;

use fil_actor_market::{
    Actor as MarketActor, DealQueryParams, GetDealActivationReturn, GetDealClientCollateralReturn,
    GetDealClientReturn, GetDealDataCommitmentReturn, GetDealLabelReturn,
    GetDealProviderCollateralReturn, GetDealProviderReturn, GetDealSectorReturn, GetDealTermReturn,
    GetDealTotalPriceReturn, GetDealVerifiedReturn, Method, EX_DEAL_EXPIRED, EX_DEAL_NOT_ACTIVATED,
};
use fil_actors_runtime::network::EPOCHS_IN_DAY;
use fil_actors_runtime::test_utils::{
    expect_abort, expect_abort_contains_message, MockRuntime, ACCOUNT_ACTOR_CODE_ID,
};
use fil_actors_runtime::ActorError;
use harness::*;

mod harness;

#[test]
fn proposal_data() {
    let start_epoch = 1000;
    let end_epoch = start_epoch + 200 * EPOCHS_IN_DAY;
    let publish_epoch = ChainEpoch::from(1);

    let rt = setup();
    rt.set_epoch(publish_epoch);
    let next_allocation_id = 1;

    let proposal = generate_deal_and_add_funds(
        &rt,
        CLIENT_ADDR,
        &MinerAddresses::default(),
        start_epoch,
        end_epoch,
    );
    rt.set_caller(*ACCOUNT_ACTOR_CODE_ID, WORKER_ADDR);
    let id = publish_deals(
        &rt,
        &MinerAddresses::default(),
        &[proposal.clone()],
        TokenAmount::zero(),
        next_allocation_id,
    )[0];

    let data: GetDealDataCommitmentReturn =
        query_deal(&rt, Method::GetDealDataCommitmentExported, id);
    assert_eq!(proposal.piece_cid, data.data);
    assert_eq!(proposal.piece_size, data.size);

    let client: GetDealClientReturn = query_deal(&rt, Method::GetDealClientExported, id);
    assert_eq!(proposal.client.id().unwrap(), client.client);

    let provider: GetDealProviderReturn = query_deal(&rt, Method::GetDealProviderExported, id);
    assert_eq!(proposal.provider.id().unwrap(), provider.provider);

    let label: GetDealLabelReturn = query_deal(&rt, Method::GetDealLabelExported, id);
    assert_eq!(proposal.label, label.label);

    let term: GetDealTermReturn = query_deal(&rt, Method::GetDealTermExported, id);
    assert_eq!(proposal.start_epoch, term.start);
    assert_eq!(proposal.duration(), term.duration);

    let price: GetDealTotalPriceReturn = query_deal(&rt, Method::GetDealTotalPriceExported, id);
    assert_eq!(proposal.total_storage_fee(), price.total_price);

    let client_collateral: GetDealClientCollateralReturn =
        query_deal(&rt, Method::GetDealClientCollateralExported, id);
    assert_eq!(proposal.client_collateral, client_collateral.collateral);

    let provider_collateral: GetDealProviderCollateralReturn =
        query_deal(&rt, Method::GetDealProviderCollateralExported, id);
    assert_eq!(proposal.provider_collateral, provider_collateral.collateral);

    let verified: GetDealVerifiedReturn = query_deal(&rt, Method::GetDealVerifiedExported, id);
    assert_eq!(proposal.verified_deal, verified.verified);

    check_state(&rt);
}

#[test]
fn activation() {
    let start_epoch = 10;
    let end_epoch = start_epoch + 180 * EPOCHS_IN_DAY;
    let publish_epoch = ChainEpoch::from(1);

    let rt = setup();
    rt.set_epoch(publish_epoch);
    let next_allocation_id = 1;

    let proposal = generate_deal_and_add_funds(
        &rt,
        CLIENT_ADDR,
        &MinerAddresses::default(),
        start_epoch,
        end_epoch,
    );
    rt.set_caller(*ACCOUNT_ACTOR_CODE_ID, WORKER_ADDR);
    let id = publish_deals(
        &rt,
        &MinerAddresses::default(),
        &[proposal],
        TokenAmount::zero(),
        next_allocation_id,
    )[0];

    let activation: GetDealActivationReturn =
        query_deal(&rt, Method::GetDealActivationExported, id);
    assert_eq!(EPOCH_UNDEFINED, activation.activated);
    assert_eq!(EPOCH_UNDEFINED, activation.terminated);
    query_deal_fails(&rt, Method::GetDealSectorExported, id, EX_DEAL_NOT_ACTIVATED);

    // activate the deal
    let activate_epoch = start_epoch - 2;
    rt.set_epoch(activate_epoch);
    let sector_number = 7;
    activate_deals(&rt, end_epoch + 1, PROVIDER_ADDR, activate_epoch, sector_number, &[id]);
    let activation: GetDealActivationReturn =
        query_deal(&rt, Method::GetDealActivationExported, id);
    assert_eq!(activate_epoch, activation.activated);
    assert_eq!(EPOCH_UNDEFINED, activation.terminated);
    assert_eq!(
        GetDealSectorReturn { sector: sector_number },
        query_deal(&rt, Method::GetDealSectorExported, id)
    );

    // terminate early
    let terminate_epoch = activate_epoch + 100;
    rt.set_epoch(terminate_epoch);
    terminate_deals(&rt, PROVIDER_ADDR, &[sector_number], &[id]);

    // terminated deal had it's state cleaned up
    expect_abort_contains_message(
        EX_DEAL_EXPIRED,
        &format!("deal {id} expired"),
        query_deal_raw(&rt, Method::GetDealActivationExported, id),
    );

    // Non-existent deal is USR_NOT_FOUND
    query_deal_fails(&rt, Method::GetDealActivationExported, id + 1, ExitCode::USR_NOT_FOUND);
    query_deal_fails(&rt, Method::GetDealSectorExported, id + 1, ExitCode::USR_NOT_FOUND);
}

fn query_deal<T: DeserializeOwned>(rt: &MockRuntime, method: Method, id: u64) -> T {
    query_deal_raw(rt, method, id).unwrap().unwrap().deserialize().unwrap()
}

fn query_deal_fails(rt: &MockRuntime, method: Method, id: u64, expected: ExitCode) {
    expect_abort(expected, query_deal_raw(rt, method, id));
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
