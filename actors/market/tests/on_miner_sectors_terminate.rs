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

    terminate_deals(&rt, PROVIDER_ADDR, &[id1]);
    assert_deals_terminated(&rt, current_epoch, &[id1]);
    assert_deals_not_terminated(&rt, &[id2, id3]);

    terminate_deals(&rt, PROVIDER_ADDR, &[id2, id3]);
    assert_deals_terminated(&rt, current_epoch, &[id1, id2, id3]);
}

#[test]
fn terminate_multiple_deals_from_multiple_providers() {
    let start_epoch = 10;
    let end_epoch = start_epoch + 200 * EPOCHS_IN_DAY;
    let sector_expiry = end_epoch + 100;
    let current_epoch = 5;

    let provider2 = Address::new_id(501);

    let rt = setup();
    rt.set_epoch(current_epoch);

    let [deal1, deal2, deal3]: [DealID; 3] = (end_epoch..end_epoch + 3)
        .map(|epoch| {
            generate_and_publish_deal(
                &rt,
                CLIENT_ADDR,
                &MinerAddresses::default(),
                start_epoch,
                epoch,
            )
        })
        .collect::<Vec<DealID>>()
        .try_into()
        .unwrap();
    let sector_number = 7; // Both providers used the same sector number
    let ret = activate_deals(
        &rt,
        sector_expiry,
        PROVIDER_ADDR,
        current_epoch,
        sector_number,
        &[deal1, deal2, deal3],
    );
    assert!(ret.activation_results.all_ok());

    let addrs = MinerAddresses { provider: provider2, ..MinerAddresses::default() };
    let deal4 = generate_and_publish_deal(&rt, CLIENT_ADDR, &addrs, start_epoch, end_epoch);
    let deal5 = generate_and_publish_deal(&rt, CLIENT_ADDR, &addrs, start_epoch, end_epoch + 1);
    let ret = activate_deals(
        &rt,
        sector_expiry,
        provider2,
        current_epoch,
        sector_number,
        &[deal4, deal5],
    );
    assert!(ret.activation_results.all_ok());

    terminate_deals(&rt, PROVIDER_ADDR, &[sector_number]);
    assert_deals_terminated(&rt, current_epoch, &[deal1, deal2, deal3]);
    assert_eq!(Vec::<DealID>::new(), get_sector_deal_ids(&rt, &PROVIDER_ADDR, sector_number));
    assert_deals_not_terminated(&rt, &[deal4, deal5]);
    assert_eq!(vec![deal4, deal5], get_sector_deal_ids(&rt, &provider2, sector_number));

    terminate_deals(&rt, provider2, &[sector_number]);
    assert_deals_terminated(&rt, current_epoch, &[deal4, deal5]);
    assert_eq!(Vec::<DealID>::new(), get_sector_deal_ids(&rt, &provider2, sector_number));
    check_state(&rt);
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
    let ret =
        activate_deals(&rt, sector_expiry, PROVIDER_ADDR, current_epoch, sector_number, &[deal1]);
    assert!(ret.activation_results.all_ok());
    terminate_deals(&rt, PROVIDER_ADDR, &[sector_number + 1]);

    let s = get_deal_state(&rt, deal1);
    assert_eq!(s.slash_epoch, -1);
    assert_eq!(vec![deal1], get_sector_deal_ids(&rt, &PROVIDER_ADDR, sector_number));
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

    let deal1 = generate_and_publish_deal(
        &rt,
        CLIENT_ADDR,
        &MinerAddresses::default(),
        start_epoch,
        end_epoch,
    );
    let deal2 = generate_and_publish_deal(
        &rt,
        CLIENT_ADDR,
        &MinerAddresses::default(),
        start_epoch,
        end_epoch + 1,
    );
    let deal3 = generate_and_publish_deal(
        &rt,
        CLIENT_ADDR,
        &MinerAddresses::default(),
        start_epoch,
        end_epoch - 1,
    );
    let sector_number = 7;
    let ret = activate_deals(
        &rt,
        sector_expiry,
        PROVIDER_ADDR,
        current_epoch,
        sector_number,
        &[deal1, deal2, deal3],
    );
    assert!(ret.activation_results.all_ok());

    let new_epoch = end_epoch - 1;
    rt.set_epoch(new_epoch);

    terminate_deals(&rt, PROVIDER_ADDR, &[sector_number]);
    assert_deals_terminated(&rt, new_epoch, &[deal1, deal2]);
    // Not cleaned up yet.
    assert_deals_not_terminated(&rt, &[deal3]);
    // All deals are removed from sector deals mapping at once.
    assert_eq!(Vec::<DealID>::new(), get_sector_deal_ids(&rt, &PROVIDER_ADDR, sector_number));
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
        &[deal1, deal2.clone()],
        TokenAmount::zero(),
        1,
    );
    assert_eq!(2, deal_ids.len());
    let sector_number = 7;
    let ret =
        activate_deals(&rt, sector_expiry, PROVIDER_ADDR, current_epoch, sector_number, &deal_ids);
    assert!(ret.activation_results.all_ok());

    let new_epoch = end_epoch - 1;
    rt.set_epoch(new_epoch);
    cron_tick(&rt);

    terminate_deals(&rt, PROVIDER_ADDR, &[sector_number]);
    assert_deals_terminated(&rt, new_epoch, &deal_ids[0..0]);
    assert_deal_deleted(&rt, deal_ids[1], deal2, sector_number);
    check_state(&rt);
}

#[test]
fn terminating_a_deal_the_second_time_does_not_change_its_slash_epoch() {
    let start_epoch = 10;
    let end_epoch = start_epoch + 200 * EPOCHS_IN_DAY;
    let sector_expiry = end_epoch + 100;
    let current_epoch = 5;

    let rt = setup();
    rt.set_epoch(current_epoch);

    let deal1 = generate_and_publish_deal(
        &rt,
        CLIENT_ADDR,
        &MinerAddresses::default(),
        start_epoch,
        end_epoch,
    );
    let sector_number = 7;
    let ret =
        activate_deals(&rt, sector_expiry, PROVIDER_ADDR, current_epoch, sector_number, &[deal1]);
    assert!(ret.activation_results.all_ok());

    // terminating the deal so slash epoch is the current epoch
    terminate_deals(&rt, PROVIDER_ADDR, &[sector_number]);

    // set a new epoch and terminate again -> however slash epoch will still be the old epoch.
    rt.set_epoch(current_epoch + 1);
    terminate_deals(&rt, PROVIDER_ADDR, &[sector_number]);
    let s = get_deal_state(&rt, deal1);
    assert_eq!(s.slash_epoch, current_epoch);
    check_state(&rt);
}

#[test]
fn terminating_new_deals_and_an_already_terminated_deal_only_terminates_the_new_deals() {
    let start_epoch = 10;
    let end_epoch = start_epoch + 200 * EPOCHS_IN_DAY;
    let sector_expiry = end_epoch + 100;
    let current_epoch = 5;

    let rt = setup();
    rt.set_epoch(current_epoch);

    // provider1 publishes deal1 and 2 and deal3 -> deal3 has the lowest endepoch
    let deals: Vec<DealID> = [end_epoch, end_epoch + 1, end_epoch - 1]
        .iter()
        .map(|&epoch| {
            generate_and_publish_deal(
                &rt,
                CLIENT_ADDR,
                &MinerAddresses::default(),
                start_epoch,
                epoch,
            )
        })
        .collect();
    let [deal1, deal2, deal3]: [DealID; 3] = deals.as_slice().try_into().unwrap();
    // Activate 1 deal
    let sector_number = 7;
    let ret = activate_deals(
        &rt,
        sector_expiry,
        PROVIDER_ADDR,
        current_epoch,
        sector_number,
        &deals[0..1],
    );
    assert!(ret.activation_results.all_ok());
    // Terminate them
    terminate_deals(&rt, PROVIDER_ADDR, &[sector_number]);

    // Activate other deals in the same sector
    let ret = activate_deals(
        &rt,
        sector_expiry,
        PROVIDER_ADDR,
        current_epoch,
        sector_number,
        &deals[1..3],
    );
    assert!(ret.activation_results.all_ok());
    // set a new epoch and terminate again
    let new_epoch = current_epoch + 1;
    rt.set_epoch(new_epoch);
    terminate_deals(&rt, PROVIDER_ADDR, &[sector_number]);

    let s1 = get_deal_state(&rt, deal1);
    assert_eq!(s1.slash_epoch, current_epoch);

    let s2 = get_deal_state(&rt, deal2);
    assert_eq!(s2.slash_epoch, new_epoch);

    let s3 = get_deal_state(&rt, deal3);
    assert_eq!(s3.slash_epoch, new_epoch);

    check_state(&rt);
}

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
    let ret =
        activate_deals(&rt, sector_expiry, PROVIDER_ADDR, current_epoch, sector_number, &[deal1]);
    assert!(ret.activation_results.all_ok());
    rt.set_epoch(end_epoch);
    terminate_deals(&rt, PROVIDER_ADDR, &[sector_number]);
    assert_deals_not_terminated(&rt, &[deal1]);

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
    let ret =
        activate_deals(&rt, sector_expiry, PROVIDER_ADDR, current_epoch, sector_number, &[deal2]);
    assert!(ret.activation_results.all_ok());
    rt.set_epoch(end_epoch + 1);
    terminate_deals(&rt, PROVIDER_ADDR, &[sector_number]);
    assert_deals_not_terminated(&rt, &[deal2]);

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
