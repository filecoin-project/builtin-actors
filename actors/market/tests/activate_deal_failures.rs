// Copyright 2019-2022 ChainSafe Systems
// SPDX-License-Identifier: Apache-2.0, MIT

use fil_actor_market::{ActivateDealsParams, Actor as MarketActor, Method, State, EX_DEAL_EXPIRED};
use fil_actors_runtime::network::EPOCHS_IN_DAY;
use fil_actors_runtime::runtime::builtins::Type;
use fil_actors_runtime::test_utils::*;
use fil_actors_runtime::BURNT_FUNDS_ACTOR_ADDR;
use fvm_ipld_encoding::ipld_block::IpldBlock;
use fvm_ipld_encoding::RawBytes;
use fvm_shared::address::Address;
use fvm_shared::deal::DealID;
use fvm_shared::error::ExitCode;
use fvm_shared::METHOD_SEND;

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
        rt.call::<MarketActor>(
            Method::ActivateDeals as u64,
            IpldBlock::serialize_cbor(&params).unwrap(),
        ),
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
        rt.call::<MarketActor>(
            Method::ActivateDeals as u64,
            IpldBlock::serialize_cbor(&params).unwrap(),
        ),
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
        rt.call::<MarketActor>(
            Method::ActivateDeals as u64,
            IpldBlock::serialize_cbor(&params).unwrap(),
        ),
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
        rt.call::<MarketActor>(
            Method::ActivateDeals as u64,
            IpldBlock::serialize_cbor(&params).unwrap(),
        ),
    );

    rt.verify();
    check_state(&rt);
}

#[test]
fn fail_when_deal_has_already_been_expired() {
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

    let deal_proposal = get_deal_proposal(&mut rt, deal_id);

    let current = end_epoch + 25;
    rt.set_epoch(current);
    rt.expect_send(
        BURNT_FUNDS_ACTOR_ADDR,
        METHOD_SEND,
        None,
        deal_proposal.provider_collateral.clone(),
        RawBytes::default(),
        ExitCode::OK,
    );

    cron_tick(&mut rt);

    assert_deal_deleted(&mut rt, deal_id, deal_proposal);

    let mut st: State = rt.get_state::<State>();
    st.next_id = deal_id + 1;

    expect_abort_contains_message(
        EX_DEAL_EXPIRED,
        "expired",
        activate_deals_raw(&mut rt, sector_expiry, PROVIDER_ADDR, 0, &[deal_id]),
    );
}
