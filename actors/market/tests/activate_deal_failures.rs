// Copyright 2019-2022 ChainSafe Systems
// SPDX-License-Identifier: Apache-2.0, MIT

use fil_actor_market::{Actor as MarketActor, Method, SectorDeals, State, EX_DEAL_EXPIRED};
use fil_actor_market::{BatchActivateDealsParams, BatchActivateDealsResult};
use fil_actors_runtime::network::EPOCHS_IN_DAY;
use fil_actors_runtime::runtime::builtins::Type;
use fil_actors_runtime::test_utils::*;
use fil_actors_runtime::BURNT_FUNDS_ACTOR_ADDR;
use fvm_ipld_encoding::ipld_block::IpldBlock;
use fvm_shared::address::Address;
use fvm_shared::deal::DealID;
use fvm_shared::error::ExitCode;
use fvm_shared::sector::RegisteredSealProof;
use fvm_shared::METHOD_SEND;

mod harness;

use harness::*;

#[test]
fn fail_when_caller_is_not_the_provider_of_the_deal() {
    let start_epoch = 10;
    let end_epoch = start_epoch + 200 * EPOCHS_IN_DAY;
    let sector_expiry = end_epoch + 100;

    let rt = setup();
    let provider2_addr = Address::new_id(201);
    let addrs = MinerAddresses { provider: provider2_addr, ..MinerAddresses::default() };
    let deal_id = generate_and_publish_deal(&rt, CLIENT_ADDR, &addrs, start_epoch, end_epoch);

    let res = batch_activate_deals_raw(
        &rt,
        PROVIDER_ADDR,
        vec![SectorDeals {
            sector_expiry,
            sector_type: RegisteredSealProof::StackedDRG8MiBV1,
            deal_ids: vec![deal_id],
        }],
        false,
    )
    .unwrap();
    let res: BatchActivateDealsResult =
        res.unwrap().deserialize().expect("BatchActivateDealsResult failed to deserialize");

    assert_eq!(res.activation_results.codes(), vec![ExitCode::USR_FORBIDDEN]);

    rt.verify();
    check_state(&rt);
}

#[test]
fn fail_when_caller_is_not_a_storage_miner_actor() {
    let rt = setup();
    rt.expect_validate_caller_type(vec![Type::Miner]);
    rt.set_caller(*ACCOUNT_ACTOR_CODE_ID, PROVIDER_ADDR);

    let sector_activation = SectorDeals {
        deal_ids: vec![],
        sector_expiry: 0,
        sector_type: RegisteredSealProof::StackedDRG8MiBV1,
    };
    let params = BatchActivateDealsParams { sectors: vec![sector_activation], compute_cid: false };

    expect_abort(
        ExitCode::USR_FORBIDDEN,
        rt.call::<MarketActor>(
            Method::BatchActivateDeals as u64,
            IpldBlock::serialize_cbor(&params).unwrap(),
        ),
    );

    rt.verify();
    check_state(&rt);
}

#[test]
fn fail_when_deal_has_not_been_published_before() {
    let rt = setup();

    let res = batch_activate_deals_raw(
        &rt,
        PROVIDER_ADDR,
        vec![SectorDeals {
            sector_type: RegisteredSealProof::StackedDRG8MiBV1,
            sector_expiry: EPOCHS_IN_DAY,
            deal_ids: vec![DealID::from(42u32)],
        }],
        false,
    )
    .unwrap();
    let res: BatchActivateDealsResult =
        res.unwrap().deserialize().expect("BatchActivateDealsResult failed to deserialize");

    assert_eq!(res.activation_results.codes(), vec![ExitCode::USR_NOT_FOUND]);

    rt.verify();
    check_state(&rt);
}

#[test]
fn fail_when_deal_has_already_been_activated() {
    let start_epoch = 10;
    let end_epoch = start_epoch + 200 * EPOCHS_IN_DAY;
    let sector_expiry = end_epoch + 100;

    let rt = setup();
    let deal_id = generate_and_publish_deal(
        &rt,
        CLIENT_ADDR,
        &MinerAddresses::default(),
        start_epoch,
        end_epoch,
    );
    activate_deals(&rt, sector_expiry, PROVIDER_ADDR, 0, &[deal_id]);

    let res = batch_activate_deals_raw(
        &rt,
        PROVIDER_ADDR,
        vec![SectorDeals {
            sector_type: RegisteredSealProof::StackedDRG8MiBV1,
            sector_expiry,
            deal_ids: vec![deal_id],
        }],
        false,
    )
    .unwrap();
    let res: BatchActivateDealsResult =
        res.unwrap().deserialize().expect("BatchActivateDealsResult failed to deserialize");

    assert_eq!(res.activation_results.codes(), vec![ExitCode::USR_ILLEGAL_ARGUMENT]);

    rt.verify();
    check_state(&rt);
}

#[test]
fn fail_when_deal_has_already_been_expired() {
    let start_epoch = 10;
    let end_epoch = start_epoch + 200 * EPOCHS_IN_DAY;
    let sector_expiry = end_epoch + 100;

    let rt = setup();
    let deal_id = generate_and_publish_deal(
        &rt,
        CLIENT_ADDR,
        &MinerAddresses::default(),
        start_epoch,
        end_epoch,
    );

    let deal_proposal = get_deal_proposal(&rt, deal_id);

    let current = end_epoch + 25;
    rt.set_epoch(current);
    rt.expect_send_simple(
        BURNT_FUNDS_ACTOR_ADDR,
        METHOD_SEND,
        None,
        deal_proposal.provider_collateral.clone(),
        None,
        ExitCode::OK,
    );

    cron_tick(&rt);

    assert_deal_deleted(&rt, deal_id, deal_proposal);

    let mut st: State = rt.get_state::<State>();
    st.next_id = deal_id + 1;

    let res = activate_deals(&rt, sector_expiry, PROVIDER_ADDR, 0, &[deal_id]);
    assert_eq!(res.activation_results.codes(), vec![EX_DEAL_EXPIRED])
}
