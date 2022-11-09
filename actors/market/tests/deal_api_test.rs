use fvm_ipld_encoding::RawBytes;
use fvm_shared::clock::ChainEpoch;
use serde::de::DeserializeOwned;

use fil_actor_market::{
    Actor as MarketActor, DealQueryParams, GetDealClientCollateralReturn, GetDealClientReturn,
    GetDealDataCommitmentReturn, GetDealEpochPriceReturn, GetDealLabelReturn,
    GetDealProviderCollateralReturn, GetDealProviderReturn, GetDealTermReturn,
    GetDealVerifiedReturn, Method,
};
use fil_actors_runtime::network::EPOCHS_IN_DAY;
use fil_actors_runtime::test_utils::{MockRuntime, ACCOUNT_ACTOR_CODE_ID};
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

    let price: GetDealEpochPriceReturn = query_deal(&mut rt, Method::GetDealEpochPriceExported, id);
    assert_eq!(proposal.storage_price_per_epoch, price.price_per_epoch);

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

fn query_deal<T: DeserializeOwned>(rt: &mut MockRuntime, method: Method, id: u64) -> T {
    let params = DealQueryParams { id };
    rt.expect_validate_caller_any();
    rt.call::<MarketActor>(method as u64, &RawBytes::serialize(params).unwrap())
        .unwrap()
        .deserialize()
        .unwrap()
}
