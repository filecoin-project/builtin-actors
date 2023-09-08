// Copyright 2019-2022 ChainSafe Systems
// SPDX-License-Identifier: Apache-2.0, MIT

use fvm_ipld_bitfield::BitField;
use std::convert::TryInto;

use fil_actor_market::{Actor as MarketActor, Method, OnMinerSectorsTerminateParams};
use fil_actors_runtime::network::EPOCHS_IN_DAY;
use fil_actors_runtime::runtime::builtins::Type;
use fil_actors_runtime::test_utils::*;
use fvm_ipld_encoding::ipld_block::IpldBlock;
use fvm_shared::address::Address;
use fvm_shared::deal::DealID;
use fvm_shared::econ::TokenAmount;
use fvm_shared::error::ExitCode;
use num_traits::Zero;

mod harness;

use harness::*;

#[test]
fn terminate_multiple_deals_from_single_provider() {
    let start_epoch = 10;
    let end_epoch = start_epoch + 200 * EPOCHS_IN_DAY;
    let sector_expiry = end_epoch + 100;
    let current_epoch = 5;

    let rt = setup();
    rt.set_epoch(current_epoch);

    // IDs are both deal and sector IDs.
    let [id1, id2, id3]: [DealID; 3] = (end_epoch..end_epoch + 3)
        .map(|epoch| {
            let id = generate_and_publish_deal(
                &rt,
                CLIENT_ADDR,
                &MinerAddresses::default(),
                start_epoch,
                epoch,
            );
            let ret = activate_deals(
                &rt,
                sector_expiry,
                PROVIDER_ADDR,
                current_epoch,
                id, // use deal ID as unique sector number
                &[id],
            );
            assert!(ret.activation_results.all_ok());
            id
        })
        .collect::<Vec<DealID>>()
        .try_into()
        .unwrap();

    let deal1 = get_deal_proposal(&rt, id1);
    let deal2 = get_deal_proposal(&rt, id2);
    let deal3 = get_deal_proposal(&rt, id3);

    terminate_deals(&rt, PROVIDER_ADDR, &[id1]);
    assert_deal_deleted(&rt, id1, &deal1, id1);
    assert_deals_not_marked_terminated(&rt, &[id2, id3]);

    terminate_deals(&rt, PROVIDER_ADDR, &[id2, id3]);
    assert_deal_deleted(&rt, id1, &deal1, id1);
    assert_deal_deleted(&rt, id1, &deal2, id2);
    assert_deal_deleted(&rt, id1, &deal3, id3);
}

#[test]
fn terminate_multiple_deals_from_multiple_providers() {
    let start_epoch = 10;
    let end_epoch = start_epoch + 200 * EPOCHS_IN_DAY;
    let sector_expiry = end_epoch + 100;
    let current_epoch = 5;

    let provider2_id = 501;
    let provider2 = Address::new_id(provider2_id);

    let rt = setup();
    rt.set_epoch(current_epoch);
    let sector_number = 7; // Both providers used the same sector number

    let addrs1 = MinerAddresses::default();
    let id0 = generate_and_publish_deal(&rt, CLIENT_ADDR, &addrs1, start_epoch, end_epoch);
    let id1 = generate_and_publish_deal(&rt, CLIENT_ADDR, &addrs1, start_epoch, end_epoch+1);
    let id2 = generate_and_publish_deal(&rt, CLIENT_ADDR, &addrs1, start_epoch, end_epoch+2);
    activate_deals_legacy(
        &rt,
        sector_expiry,
        PROVIDER_ADDR,
        current_epoch,
        sector_number,
        &[id0, id1, id2],
    );

    let addrs2 = MinerAddresses { provider: provider2, ..MinerAddresses::default() };
    let id3 = generate_and_publish_deal(&rt, CLIENT_ADDR, &addrs2, start_epoch, end_epoch);
    let id4 = generate_and_publish_deal(&rt, CLIENT_ADDR, &addrs2, start_epoch, end_epoch + 1);
    activate_deals_legacy(
        &rt,
        sector_expiry,
        provider2,
        current_epoch,
        sector_number,
        &[id3, id4],
    );

    let deals = &[id0, id1, id2, id3, id4].iter().map(|id| get_deal_proposal(&rt, *id)).collect::<Vec<_>>();

    terminate_deals_and_assert_balances(&rt, CLIENT_ADDR, PROVIDER_ADDR, &[sector_number]);
    assert_deal_deleted(&rt, id0, &deals[0], sector_number);
    assert_deal_deleted(&rt, id1, &deals[1], sector_number);
    assert_deal_deleted(&rt, id2, &deals[2], sector_number);
    assert_deals_not_marked_terminated(&rt, &[id3, id4]);

    terminate_deals_and_assert_balances(&rt, CLIENT_ADDR, provider2, &[sector_number]);
    assert_deal_deleted(&rt, id3, &deals[3], sector_number);
    assert_deal_deleted(&rt, id4, &deals[4], sector_number);
}

#[test]
fn ignore_sector_that_does_not_exist() {
    let start_epoch = 10;
    let end_epoch = start_epoch + 200 * EPOCHS_IN_DAY;
    let sector_expiry = end_epoch + 100;
    let current_epoch = 5;
    let sector_number = 7;

    let rt = setup();
    rt.set_epoch(current_epoch);

    let deal1 = generate_and_publish_deal(
        &rt,
        CLIENT_ADDR,
        &MinerAddresses::default(),
        start_epoch,
        end_epoch,
    );
    let ret = activate_deals_legacy(
        &rt,
        sector_expiry,
        PROVIDER_ADDR,
        current_epoch,
        sector_number,
        &[deal1],
    );
    assert!(ret.activation_results.all_ok());
    terminate_deals(&rt, PROVIDER_ADDR, &[sector_number + 1]);

    let s = get_deal_state(&rt, deal1);
    assert_eq!(s.slash_epoch, -1);
    assert_eq!(vec![deal1], get_sector_deal_ids(&rt, PROVIDER_ID, &[sector_number]));
    check_state(&rt);
}

#[test]
fn terminate_valid_deals_along_with_just_expired_deal() {
    let start_epoch = 10;
    let end_epoch = start_epoch + 200 * EPOCHS_IN_DAY;
    let sector_expiry = end_epoch + 100;
    let current_epoch = 5;

    let rt = setup();
    rt.set_epoch(current_epoch);

    let id0 = generate_and_publish_deal(
        &rt,
        CLIENT_ADDR,
        &MinerAddresses::default(),
        start_epoch,
        end_epoch,
    );
    let id1 = generate_and_publish_deal(
        &rt,
        CLIENT_ADDR,
        &MinerAddresses::default(),
        start_epoch,
        end_epoch + 1,
    );
    let id2 = generate_and_publish_deal(
        &rt,
        CLIENT_ADDR,
        &MinerAddresses::default(),
        start_epoch,
        end_epoch - 1,
    );
    let sector_number = 7;
    let ret = activate_deals_legacy(
        &rt,
        sector_expiry,
        PROVIDER_ADDR,
        current_epoch,
        sector_number,
        &[id0, id1, id2],
    );
    assert!(ret.activation_results.all_ok());
    let deals = &[id0, id1, id2].iter().map(|id| get_deal_proposal(&rt, *id)).collect::<Vec<_>>();

    let new_epoch = end_epoch - 1;
    rt.set_epoch(new_epoch);

    terminate_deals_and_assert_balances(&rt, CLIENT_ADDR, PROVIDER_ADDR, &[sector_number]);
    assert_deal_deleted(&rt, id0, &deals[0], sector_number);
    assert_deal_deleted(&rt, id1, &deals[1], sector_number);
    assert_deal_deleted(&rt, id1, &deals[2], sector_number);
    // All deals are removed from sector deals mapping at once.
    assert_eq!(Vec::<DealID>::new(), get_sector_deal_ids(&rt, PROVIDER_ID, &[sector_number]));
    check_state(&rt);
}

#[test]
fn terminate_valid_deals_along_with_expired_and_cleaned_up_deal() {
    let start_epoch = 10;
    let end_epoch = start_epoch + 200 * EPOCHS_IN_DAY;
    let sector_expiry = end_epoch + 100;

    let rt = setup();
    let current_epoch = rt.set_epoch(5);

    let deal1 = generate_deal_and_add_funds(
        &rt,
        CLIENT_ADDR,
        &MinerAddresses::default(),
        start_epoch,
        end_epoch,
    );
    let deal2 = generate_deal_and_add_funds(
        &rt,
        CLIENT_ADDR,
        &MinerAddresses::default(),
        start_epoch,
        end_epoch - 10, // Expires earlier
    );

    rt.set_caller(*ACCOUNT_ACTOR_CODE_ID, WORKER_ADDR);
    let deal_ids = publish_deals(
        &rt,
        &MinerAddresses::default(),
        &[deal1.clone(), deal2.clone()],
        TokenAmount::zero(),
        1,
    );
    assert_eq!(2, deal_ids.len());
    let sector_number = 7;
    let ret = activate_deals_legacy(
        &rt,
        sector_expiry,
        PROVIDER_ADDR,
        current_epoch,
        sector_number,
        &deal_ids,
    );
    assert!(ret.activation_results.all_ok());

    let new_epoch = end_epoch - 1;
    rt.set_epoch(new_epoch);
    cron_tick(&rt);
    // expired deal deleted normally
    assert_deal_deleted(&rt, deal_ids[1], &deal2, sector_number);
    assert_deals_not_marked_terminated(&rt, &deal_ids[0..0]);

    terminate_deals_and_assert_balances(&rt, CLIENT_ADDR, PROVIDER_ADDR, &[sector_number]);
    // terminated deal deleted
    assert_deal_deleted(&rt, deal_ids[0], &deal1, sector_number);

    // terminated deal has a dangling deal op, normally expired deal doesn't
    check_state(&rt);
}

// Converted from: https://github.com/filecoin-project/specs-actors/blob/d56b240af24517443ce1f8abfbdab7cb22d331f1/actors/builtin/market/market_test.go#L1415
#[test]
fn do_not_terminate_deal_if_end_epoch_is_equal_to_or_less_than_current_epoch() {
    let start_epoch = 10;
    let end_epoch = start_epoch + 200 * EPOCHS_IN_DAY;
    let sector_expiry = end_epoch + 100;
    let current_epoch = 5;

    let rt = setup();
    rt.set_epoch(current_epoch);

    // deal1 has endepoch equal to current epoch when terminate is called
    let deal1 = generate_and_publish_deal(
        &rt,
        CLIENT_ADDR,
        &MinerAddresses::default(),
        start_epoch,
        end_epoch,
    );
    let sector_number = 7;
    let ret = activate_deals_legacy(
        &rt,
        sector_expiry,
        PROVIDER_ADDR,
        current_epoch,
        sector_number,
        &[deal1],
    );
    assert!(ret.activation_results.all_ok());
    rt.set_epoch(end_epoch);
    terminate_deals_and_assert_balances(&rt, CLIENT_ADDR, PROVIDER_ADDR, &[sector_number]);
    assert_deals_not_marked_terminated(&rt, &[deal1]);

    // deal2 has end epoch less than current epoch when terminate is called
    rt.set_epoch(current_epoch);
    let deal2 = generate_and_publish_deal(
        &rt,
        CLIENT_ADDR,
        &MinerAddresses::default(),
        start_epoch + 1,
        end_epoch,
    );
    let sector_number = sector_number + 1;
    let ret = activate_deals_legacy(
        &rt,
        sector_expiry,
        PROVIDER_ADDR,
        current_epoch,
        sector_number,
        &[deal2],
    );
    assert!(ret.activation_results.all_ok());
    rt.set_epoch(end_epoch + 1);
    terminate_deals_and_assert_balances(&rt, CLIENT_ADDR, PROVIDER_ADDR, &[sector_number]);
    assert_deals_not_marked_terminated(&rt, &[deal2]);

    check_state(&rt);
}

#[test]
fn fail_when_caller_is_not_a_storage_miner_actor() {
    let rt = setup();
    rt.expect_validate_caller_type(vec![Type::Miner]);
    rt.set_caller(*ACCOUNT_ACTOR_CODE_ID, PROVIDER_ADDR);
    let params =
        OnMinerSectorsTerminateParams { epoch: *rt.epoch.borrow(), sectors: BitField::new() };

    assert_eq!(
        ExitCode::USR_FORBIDDEN,
        rt.call::<MarketActor>(
            Method::OnMinerSectorsTerminate as u64,
            IpldBlock::serialize_cbor(&params).unwrap(),
        )
        .unwrap_err()
        .exit_code()
    );

    check_state(&rt);
}
