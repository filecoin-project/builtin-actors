// Copyright 2019-2022 ChainSafe Systems
// SPDX-License-Identifier: Apache-2.0, MIT

mod harness;
use fil_actor_market::policy::detail::deal_weight;
use fil_actor_market::{Actor as MarketActor, Method, SectorDeals, VerifyDealsForActivationParams};
use fil_actors_runtime::runtime::builtins::Type;
use fil_actors_runtime::test_utils::{
    expect_abort, expect_abort_contains_message, ACCOUNT_ACTOR_CODE_ID, MINER_ACTOR_CODE_ID,
};
use fil_actors_runtime::EPOCHS_IN_DAY;
use fvm_ipld_encoding::RawBytes;
use fvm_shared::address::Address;
use fvm_shared::bigint::BigInt;
use fvm_shared::clock::ChainEpoch;
use fvm_shared::error::ExitCode;
use harness::*;
use num_traits::Zero;

const START_EPOCH: ChainEpoch = 10;
const END_EPOCH: ChainEpoch = 200 * EPOCHS_IN_DAY;
const SECTOR_EXPIRY: ChainEpoch = END_EPOCH + 200;
const MINER_ADDRESSES: MinerAddresses = MinerAddresses {
    owner: OWNER_ADDR,
    worker: WORKER_ADDR,
    provider: PROVIDER_ADDR,
    control: vec![],
};

#[test]
fn verify_deal_and_get_deal_weight_for_unverified_deal_proposal() {
    let mut rt = setup();
    let deal_id =
        generate_and_publish_deal(&mut rt, CLIENT_ADDR, &MINER_ADDRESSES, START_EPOCH, END_EPOCH);
    let deal_proposal = get_deal_proposal(&mut rt, deal_id);

    let response = verify_deals_for_activation(
        &mut rt,
        PROVIDER_ADDR,
        vec![SectorDeals { sector_expiry: SECTOR_EXPIRY, deal_ids: vec![deal_id] }],
    );

    assert_eq!(1, response.sectors.len());
    assert_eq!(BigInt::zero(), response.sectors[0].verified_deal_weight);
    assert_eq!(deal_weight(&deal_proposal), response.sectors[0].deal_weight);

    check_state(&rt);
}

#[test]
fn verify_deal_and_get_deal_weight_for_verified_deal_proposal() {
    let mut rt = setup();
    let deal_id = generate_and_publish_verified_deal(
        &mut rt,
        CLIENT_ADDR,
        &MINER_ADDRESSES,
        START_EPOCH,
        END_EPOCH,
    );
    let deal_proposal = get_deal_proposal(&mut rt, deal_id);

    let response = verify_deals_for_activation(
        &mut rt,
        PROVIDER_ADDR,
        vec![SectorDeals { sector_expiry: SECTOR_EXPIRY, deal_ids: vec![deal_id] }],
    );

    assert_eq!(1, response.sectors.len());
    assert_eq!(deal_weight(&deal_proposal), response.sectors[0].verified_deal_weight);
    assert_eq!(BigInt::zero(), response.sectors[0].deal_weight);

    check_state(&rt);
}

#[test]
fn verification_and_weights_for_verified_and_unverified_deals() {
    let mut rt = setup();
    let mut create_deal = |end_epoch, verified| {
        let mut deal = generate_deal_and_add_funds(
            &mut rt,
            CLIENT_ADDR,
            &MINER_ADDRESSES,
            START_EPOCH,
            end_epoch,
        );
        deal.verified_deal = verified;
        deal
    };

    let verified_deal_1 = create_deal(END_EPOCH, true);
    let verified_deal_2 = create_deal(END_EPOCH + 1, true);
    let unverified_deal_1 = create_deal(END_EPOCH + 2, false);
    let unverified_deal_2 = create_deal(END_EPOCH + 3, false);

    rt.set_caller(*ACCOUNT_ACTOR_CODE_ID, WORKER_ADDR);
    let deal_ids = publish_deals(
        &mut rt,
        &MINER_ADDRESSES,
        &[
            verified_deal_1.clone(),
            verified_deal_2.clone(),
            unverified_deal_1.clone(),
            unverified_deal_2.clone(),
        ],
    );

    let response = verify_deals_for_activation(
        &mut rt,
        PROVIDER_ADDR,
        vec![SectorDeals { sector_expiry: SECTOR_EXPIRY, deal_ids }],
    );

    let verified_weight = deal_weight(&verified_deal_1) + deal_weight(&verified_deal_2);
    let unverified_weight = deal_weight(&unverified_deal_1) + deal_weight(&unverified_deal_2);

    assert_eq!(1, response.sectors.len());
    assert_eq!(verified_weight, response.sectors[0].verified_deal_weight);
    assert_eq!(unverified_weight, response.sectors[0].deal_weight);

    check_state(&rt);
}

#[test]
fn fail_when_caller_is_not_a_storage_miner_actor() {
    let mut rt = setup();
    let deal_id =
        generate_and_publish_deal(&mut rt, CLIENT_ADDR, &MINER_ADDRESSES, START_EPOCH, END_EPOCH);

    rt.set_caller(*ACCOUNT_ACTOR_CODE_ID, WORKER_ADDR);
    rt.expect_validate_caller_type(vec![Type::Miner]);

    let params = VerifyDealsForActivationParams {
        sectors: vec![SectorDeals { sector_expiry: SECTOR_EXPIRY, deal_ids: vec![deal_id] }],
    };
    expect_abort(
        ExitCode::USR_FORBIDDEN,
        rt.call::<MarketActor>(
            Method::VerifyDealsForActivation as u64,
            &RawBytes::serialize(params).unwrap(),
        ),
    );

    rt.verify();
    check_state(&rt);
}

#[test]
fn fail_when_deal_proposal_is_not_found() {
    let mut rt = setup();

    let params = VerifyDealsForActivationParams {
        sectors: vec![SectorDeals { sector_expiry: SECTOR_EXPIRY, deal_ids: vec![1] }],
    };
    rt.set_caller(*MINER_ACTOR_CODE_ID, PROVIDER_ADDR);
    rt.expect_validate_caller_type(vec![Type::Miner]);
    expect_abort(
        ExitCode::USR_NOT_FOUND,
        rt.call::<MarketActor>(
            Method::VerifyDealsForActivation as u64,
            &RawBytes::serialize(params).unwrap(),
        ),
    );

    rt.verify();
    check_state(&rt);
}

#[test]
fn fail_when_caller_is_not_the_provider() {
    let mut rt = setup();
    let deal_id =
        generate_and_publish_deal(&mut rt, CLIENT_ADDR, &MINER_ADDRESSES, START_EPOCH, END_EPOCH);

    rt.set_caller(*MINER_ACTOR_CODE_ID, Address::new_id(205));
    rt.expect_validate_caller_type(vec![Type::Miner]);

    let params = VerifyDealsForActivationParams {
        sectors: vec![SectorDeals { sector_expiry: SECTOR_EXPIRY, deal_ids: vec![deal_id] }],
    };
    expect_abort(
        ExitCode::USR_FORBIDDEN,
        rt.call::<MarketActor>(
            Method::VerifyDealsForActivation as u64,
            &RawBytes::serialize(params).unwrap(),
        ),
    );

    rt.verify();
    check_state(&rt);
}

#[test]
fn fail_when_current_epoch_is_greater_than_proposal_start_epoch() {
    let mut rt = setup();
    let deal_id =
        generate_and_publish_deal(&mut rt, CLIENT_ADDR, &MINER_ADDRESSES, START_EPOCH, END_EPOCH);
    rt.set_epoch(START_EPOCH + 1);

    rt.set_caller(*MINER_ACTOR_CODE_ID, PROVIDER_ADDR);
    rt.expect_validate_caller_type(vec![Type::Miner]);

    let params = VerifyDealsForActivationParams {
        sectors: vec![SectorDeals { sector_expiry: SECTOR_EXPIRY, deal_ids: vec![deal_id] }],
    };
    expect_abort(
        ExitCode::USR_ILLEGAL_ARGUMENT,
        rt.call::<MarketActor>(
            Method::VerifyDealsForActivation as u64,
            &RawBytes::serialize(params).unwrap(),
        ),
    );

    rt.verify();
    check_state(&rt);
}

#[test]
fn fail_when_deal_end_epoch_is_greater_than_sector_expiration() {
    let mut rt = setup();
    let deal_id =
        generate_and_publish_deal(&mut rt, CLIENT_ADDR, &MINER_ADDRESSES, START_EPOCH, END_EPOCH);

    rt.set_caller(*MINER_ACTOR_CODE_ID, PROVIDER_ADDR);
    rt.expect_validate_caller_type(vec![Type::Miner]);

    let params = VerifyDealsForActivationParams {
        sectors: vec![SectorDeals { sector_expiry: END_EPOCH - 1, deal_ids: vec![deal_id] }],
    };
    expect_abort(
        ExitCode::USR_ILLEGAL_ARGUMENT,
        rt.call::<MarketActor>(
            Method::VerifyDealsForActivation as u64,
            &RawBytes::serialize(params).unwrap(),
        ),
    );

    rt.verify();
    check_state(&rt);
}

#[test]
fn fail_when_the_same_deal_id_is_passed_multiple_times() {
    let mut rt = setup();
    let deal_id =
        generate_and_publish_deal(&mut rt, CLIENT_ADDR, &MINER_ADDRESSES, START_EPOCH, END_EPOCH);

    rt.set_caller(*MINER_ACTOR_CODE_ID, PROVIDER_ADDR);
    rt.expect_validate_caller_type(vec![Type::Miner]);

    let params = VerifyDealsForActivationParams {
        sectors: vec![SectorDeals {
            sector_expiry: SECTOR_EXPIRY,
            deal_ids: vec![deal_id, deal_id],
        }],
    };
    expect_abort_contains_message(
        ExitCode::USR_ILLEGAL_ARGUMENT,
        "multiple times",
        rt.call::<MarketActor>(
            Method::VerifyDealsForActivation as u64,
            &RawBytes::serialize(params).unwrap(),
        ),
    );

    rt.verify();
    check_state(&rt);
}
