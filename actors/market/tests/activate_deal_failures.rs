// Copyright 2019-2022 ChainSafe Systems
// SPDX-License-Identifier: Apache-2.0, MIT

use fil_actor_market::{ActivateDealsParams, Actor as MarketActor, Method};
use fil_actors_runtime_common::network::EPOCHS_IN_DAY;
use fil_actors_runtime_common::runtime::builtins::Type;
use fil_actors_runtime_common::test_utils::*;
use fvm_ipld_encoding::RawBytes;
use fvm_shared::address::Address;
use fvm_shared::deal::DealID;
use fvm_shared::error::ExitCode;

mod harness;
use harness::*;

#[test]
fn fail_when_caller_is_not_the_provider_of_the_deal() {
    let start_epoch = 10;
    let end_epoch = start_epoch + 200 * EPOCHS_IN_DAY;
    let sector_expiry = end_epoch + 100;

    let mut rt = setup();
    let provider2_addr = Address::new_id(201);
    let addrs = MinerAddresses { provider: provider2_addr, ..MinerAddresses::default() };
    let deal_id = generate_and_publish_deal(&mut rt, CLIENT_ADDR, &addrs, start_epoch, end_epoch);

    let params = ActivateDealsParams { deal_ids: vec![deal_id], sector_expiry };

    rt.expect_validate_caller_type(vec![Type::Miner]);
    rt.set_caller(*MINER_ACTOR_CODE_ID, PROVIDER_ADDR);
    expect_abort(
        ExitCode::USR_FORBIDDEN,
        rt.call::<MarketActor>(Method::ActivateDeals as u64, &RawBytes::serialize(params).unwrap()),
    );

    rt.verify();
    check_state(&rt);
}

#[test]
fn fail_when_caller_is_not_a_storage_miner_actor() {
    let mut rt = setup();
    rt.expect_validate_caller_type(vec![Type::Miner]);
    rt.set_caller(*ACCOUNT_ACTOR_CODE_ID, PROVIDER_ADDR);

    let params = ActivateDealsParams { deal_ids: vec![], sector_expiry: 0 };
    expect_abort(
        ExitCode::USR_FORBIDDEN,
        rt.call::<MarketActor>(Method::ActivateDeals as u64, &RawBytes::serialize(params).unwrap()),
    );

    rt.verify();
    check_state(&rt);
}

#[test]
fn fail_when_deal_has_not_been_published_before() {
    let mut rt = setup();
    let params = ActivateDealsParams { deal_ids: vec![DealID::from(42u32)], sector_expiry: 0 };

    rt.expect_validate_caller_type(vec![Type::Miner]);
    rt.set_caller(*MINER_ACTOR_CODE_ID, PROVIDER_ADDR);
    expect_abort(
        ExitCode::USR_NOT_FOUND,
        rt.call::<MarketActor>(Method::ActivateDeals as u64, &RawBytes::serialize(params).unwrap()),
    );

    rt.verify();
    check_state(&rt);
}

#[test]
fn fail_when_deal_has_already_been_activated() {
    let start_epoch = 10;
    let end_epoch = start_epoch + 200 * EPOCHS_IN_DAY;
    let sector_expiry = end_epoch + 100;

    let mut rt = setup();
    let deal_id = generate_and_publish_deal(
        &mut rt,
        CLIENT_ADDR,
        &MinerAddresses::default(),
        start_epoch,
        end_epoch,
    );
    activate_deals(&mut rt, sector_expiry, PROVIDER_ADDR, 0, &[deal_id]);

    rt.expect_validate_caller_type(vec![Type::Miner]);
    rt.set_caller(*MINER_ACTOR_CODE_ID, PROVIDER_ADDR);
    let params = ActivateDealsParams { deal_ids: vec![deal_id], sector_expiry };
    expect_abort(
        ExitCode::USR_ILLEGAL_ARGUMENT,
        rt.call::<MarketActor>(Method::ActivateDeals as u64, &RawBytes::serialize(params).unwrap()),
    );

    rt.verify();
    check_state(&rt);
}
