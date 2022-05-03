// Copyright 2019-2022 ChainSafe Systems
// SPDX-License-Identifier: Apache-2.0, MIT

use std::convert::TryInto;

use fil_actor_market::{Actor as MarketActor, Method, OnMinerSectorsTerminateParams};
use fil_actors_runtime::network::EPOCHS_IN_DAY;
use fil_actors_runtime::runtime::Policy;
use fil_actors_runtime::test_utils::*;
use fvm_ipld_encoding::RawBytes;
use fvm_shared::address::Address;
use fvm_shared::deal::DealID;
use fvm_shared::error::ExitCode;

mod harness;
use harness::*;

#[cfg(test)]
mod on_miner_sectors_terminate {
    use super::*;

    // Converted from https://github.com/filecoin-project/specs-actors/blob/d56b240af24517443ce1f8abfbdab7cb22d331f1/actors/builtin/market/market_test.go#L1274
    #[test]
    fn terminate_multiple_deals_from_multiple_providers() {
        let start_epoch = 10;
        let end_epoch = start_epoch + 200 * EPOCHS_IN_DAY;
        let sector_expiry = end_epoch + 100;
        let current_epoch = 5;

        let provider2 = Address::new_id(501);

        let mut rt = setup();
        rt.set_epoch(current_epoch);

        let [deal1, deal2, deal3]: [DealID; 3] = (end_epoch..end_epoch + 3)
            .map(|epoch| {
                generate_and_publish_deal(
                    &mut rt,
                    CLIENT_ADDR,
                    &MinerAddresses::default(),
                    start_epoch,
                    epoch,
                )
            })
            .collect::<Vec<DealID>>()
            .try_into()
            .unwrap();
        activate_deals(
            &mut rt,
            sector_expiry,
            PROVIDER_ADDR,
            current_epoch,
            &[deal1, deal2, deal3],
        );

        let addrs = MinerAddresses { provider: provider2, ..MinerAddresses::default() };
        let deal4 = generate_and_publish_deal(&mut rt, CLIENT_ADDR, &addrs, start_epoch, end_epoch);
        let deal5 =
            generate_and_publish_deal(&mut rt, CLIENT_ADDR, &addrs, start_epoch, end_epoch + 1);
        activate_deals(&mut rt, sector_expiry, provider2, current_epoch, &[deal4, deal5]);

        terminate_deals(&mut rt, PROVIDER_ADDR, &[deal1]);
        assert_deals_terminated(&mut rt, current_epoch, &[deal1]);
        assert_deals_not_terminated(&mut rt, &[deal2, deal3, deal4, deal5]);

        terminate_deals(&mut rt, provider2, &[deal5]);
        assert_deals_terminated(&mut rt, current_epoch, &[deal5]);
        assert_deals_not_terminated(&mut rt, &[deal2, deal3, deal4]);

        terminate_deals(&mut rt, PROVIDER_ADDR, &[deal2, deal3]);
        assert_deals_terminated(&mut rt, current_epoch, &[deal2, deal3]);
        assert_deals_not_terminated(&mut rt, &[deal4]);

        terminate_deals(&mut rt, provider2, &[deal4]);
        assert_deals_terminated(&mut rt, current_epoch, &[deal4]);
    }

    // Converted from: https://github.com/filecoin-project/specs-actors/blob/d56b240af24517443ce1f8abfbdab7cb22d331f1/actors/builtin/market/market_test.go#L1312
    #[test]
    fn ignore_deal_proposal_that_does_not_exist() {
        let start_epoch = 10;
        let end_epoch = start_epoch + 200 * EPOCHS_IN_DAY;
        let sector_expiry = end_epoch + 100;
        let current_epoch = 5;

        let mut rt = setup();
        rt.set_epoch(current_epoch);

        let deal1 = generate_and_publish_deal(
            &mut rt,
            CLIENT_ADDR,
            &MinerAddresses::default(),
            start_epoch,
            end_epoch,
        );
        activate_deals(&mut rt, sector_expiry, PROVIDER_ADDR, current_epoch, &[deal1]);

        terminate_deals(&mut rt, PROVIDER_ADDR, &[deal1, 42]);

        let s = get_deal_state(&mut rt, deal1);
        assert_eq!(s.slash_epoch, current_epoch);
    }

    // Converted from: https://github.com/filecoin-project/specs-actors/blob/d56b240af24517443ce1f8abfbdab7cb22d331f1/actors/builtin/market/market_test.go#L1326
    #[test]
    fn terminate_valid_deals_along_with_just_expired_deal() {
        let start_epoch = 10;
        let end_epoch = start_epoch + 200 * EPOCHS_IN_DAY;
        let sector_expiry = end_epoch + 100;
        let current_epoch = 5;

        let mut rt = setup();
        rt.set_epoch(current_epoch);

        let deal1 = generate_and_publish_deal(
            &mut rt,
            CLIENT_ADDR,
            &MinerAddresses::default(),
            start_epoch,
            end_epoch,
        );
        let deal2 = generate_and_publish_deal(
            &mut rt,
            CLIENT_ADDR,
            &MinerAddresses::default(),
            start_epoch,
            end_epoch + 1,
        );
        let deal3 = generate_and_publish_deal(
            &mut rt,
            CLIENT_ADDR,
            &MinerAddresses::default(),
            start_epoch,
            end_epoch - 1,
        );
        activate_deals(
            &mut rt,
            sector_expiry,
            PROVIDER_ADDR,
            current_epoch,
            &[deal1, deal2, deal3],
        );

        let new_epoch = end_epoch - 1;
        rt.set_epoch(new_epoch);

        terminate_deals(&mut rt, PROVIDER_ADDR, &[deal1, deal2, deal3]);
        assert_deals_terminated(&mut rt, new_epoch, &[deal1, deal2]);
        assert_deals_not_terminated(&mut rt, &[deal3]);
    }
    // Converted from: https://github.com/filecoin-project/specs-actors/blob/d56b240af24517443ce1f8abfbdab7cb22d331f1/actors/builtin/market/market_test.go#L1346
    #[test]
    fn terminate_valid_deals_along_with_expired_and_cleaned_up_deal() {
        let deal_updates_interval = Policy::default().deal_updates_interval;
        let start_epoch = 10;
        let end_epoch = start_epoch + 200 * EPOCHS_IN_DAY;
        let sector_expiry = end_epoch + 100;
        let current_epoch = 5;

        let mut rt = setup();
        rt.set_epoch(current_epoch);

        let deal1 = generate_deal_and_add_funds(
            &mut rt,
            CLIENT_ADDR,
            &MinerAddresses::default(),
            start_epoch,
            end_epoch,
        );
        let deal2 = generate_deal_and_add_funds(
            &mut rt,
            CLIENT_ADDR,
            &MinerAddresses::default(),
            start_epoch,
            end_epoch - deal_updates_interval,
        );

        rt.set_caller(*ACCOUNT_ACTOR_CODE_ID, WORKER_ADDR);
        let deal_ids = publish_deals(&mut rt, &MinerAddresses::default(), &[deal1, deal2.clone()]);
        activate_deals(&mut rt, sector_expiry, PROVIDER_ADDR, current_epoch, &deal_ids);

        let new_epoch = end_epoch - 1;
        rt.set_epoch(new_epoch);
        cron_tick(&mut rt);

        terminate_deals(&mut rt, PROVIDER_ADDR, &deal_ids);
        assert_deals_terminated(&mut rt, new_epoch, &deal_ids[0..0]);
        assert_deal_deleted(&mut rt, deal_ids[1], deal2);
    }

    // Converted from: https://github.com/filecoin-project/specs-actors/blob/d56b240af24517443ce1f8abfbdab7cb22d331f1/actors/builtin/market/market_test.go#L1369
    #[test]
    fn terminating_a_deal_the_second_time_does_not_change_its_slash_epoch() {
        let start_epoch = 10;
        let end_epoch = start_epoch + 200 * EPOCHS_IN_DAY;
        let sector_expiry = end_epoch + 100;
        let current_epoch = 5;

        let mut rt = setup();
        rt.set_epoch(current_epoch);

        let deal1 = generate_and_publish_deal(
            &mut rt,
            CLIENT_ADDR,
            &MinerAddresses::default(),
            start_epoch,
            end_epoch,
        );
        activate_deals(&mut rt, sector_expiry, PROVIDER_ADDR, current_epoch, &[deal1]);

        // terminating the deal so slash epoch is the current epoch
        terminate_deals(&mut rt, PROVIDER_ADDR, &[deal1]);

        // set a new epoch and terminate again -> however slash epoch will still be the old epoch.
        rt.set_epoch(current_epoch + 1);
        terminate_deals(&mut rt, PROVIDER_ADDR, &[deal1]);
        let s = get_deal_state(&mut rt, deal1);
        assert_eq!(s.slash_epoch, current_epoch);
    }

    // Converted from: https://github.com/filecoin-project/specs-actors/blob/d56b240af24517443ce1f8abfbdab7cb22d331f1/actors/builtin/market/market_test.go#L1387
    #[test]
    fn terminating_new_deals_and_an_already_terminated_deal_only_terminates_the_new_deals() {
        let start_epoch = 10;
        let end_epoch = start_epoch + 200 * EPOCHS_IN_DAY;
        let sector_expiry = end_epoch + 100;
        let current_epoch = 5;

        let mut rt = setup();
        rt.set_epoch(current_epoch);

        // provider1 publishes deal1 and 2 and deal3 -> deal3 has the lowest endepoch
        let deals: Vec<DealID> = [end_epoch, end_epoch + 1, end_epoch - 1]
            .iter()
            .map(|&epoch| {
                generate_and_publish_deal(
                    &mut rt,
                    CLIENT_ADDR,
                    &MinerAddresses::default(),
                    start_epoch,
                    epoch,
                )
            })
            .collect();
        let [deal1, deal2, deal3]: [DealID; 3] = deals.as_slice().try_into().unwrap();
        activate_deals(&mut rt, sector_expiry, PROVIDER_ADDR, current_epoch, &deals);

        // terminating the deal so slash epoch is the current epoch
        terminate_deals(&mut rt, PROVIDER_ADDR, &[deal1]);

        // set a new epoch and terminate again -> however slash epoch will still be the old epoch.
        let new_epoch = current_epoch + 1;
        rt.set_epoch(new_epoch);
        terminate_deals(&mut rt, PROVIDER_ADDR, &deals);

        let s1 = get_deal_state(&mut rt, deal1);
        assert_eq!(s1.slash_epoch, current_epoch);

        let s2 = get_deal_state(&mut rt, deal2);
        assert_eq!(s2.slash_epoch, new_epoch);

        let s3 = get_deal_state(&mut rt, deal3);
        assert_eq!(s3.slash_epoch, new_epoch);
    }

    // Converted from: https://github.com/filecoin-project/specs-actors/blob/d56b240af24517443ce1f8abfbdab7cb22d331f1/actors/builtin/market/market_test.go#L1415
    #[test]
    fn do_not_terminate_deal_if_end_epoch_is_equal_to_or_less_than_current_epoch() {
        let start_epoch = 10;
        let end_epoch = start_epoch + 200 * EPOCHS_IN_DAY;
        let sector_expiry = end_epoch + 100;
        let current_epoch = 5;

        let mut rt = setup();
        rt.set_epoch(current_epoch);

        // deal1 has endepoch equal to current epoch when terminate is called
        let deal1 = generate_and_publish_deal(
            &mut rt,
            CLIENT_ADDR,
            &MinerAddresses::default(),
            start_epoch,
            end_epoch,
        );
        activate_deals(&mut rt, sector_expiry, PROVIDER_ADDR, current_epoch, &[deal1]);
        rt.set_epoch(end_epoch);
        terminate_deals(&mut rt, PROVIDER_ADDR, &[deal1]);
        assert_deals_not_terminated(&mut rt, &[deal1]);

        // deal2 has end epoch less than current epoch when terminate is called
        rt.set_epoch(current_epoch);
        let deal2 = generate_and_publish_deal(
            &mut rt,
            CLIENT_ADDR,
            &MinerAddresses::default(),
            start_epoch + 1,
            end_epoch,
        );
        activate_deals(&mut rt, sector_expiry, PROVIDER_ADDR, current_epoch, &[deal2]);
        rt.set_epoch(end_epoch + 1);
        terminate_deals(&mut rt, PROVIDER_ADDR, &[deal2]);
        assert_deals_not_terminated(&mut rt, &[deal2]);
    }

    // Converted from: https://github.com/filecoin-project/specs-actors/blob/master/actors/builtin/market/market_test.go#L1436
    #[test]
    fn fail_when_caller_is_not_a_storage_miner_actor() {
        let mut rt = setup();
        rt.expect_validate_caller_type(vec![*MINER_ACTOR_CODE_ID]);
        rt.set_caller(*ACCOUNT_ACTOR_CODE_ID, PROVIDER_ADDR);
        let params = OnMinerSectorsTerminateParams { epoch: rt.epoch, deal_ids: vec![] };

        // XXX: Which exit code is correct: SYS_FORBIDDEN(8) or USR_FORBIDDEN(18)?
        assert_eq!(
            ExitCode::USR_FORBIDDEN,
            rt.call::<MarketActor>(
                Method::OnMinerSectorsTerminate as u64,
                &RawBytes::serialize(params).unwrap(),
            )
            .unwrap_err()
            .exit_code()
        );
    }

    // Converted from: https://github.com/filecoin-project/specs-actors/blob/master/actors/builtin/market/market_test.go#L1448
    #[test]
    fn fail_when_caller_is_not_the_provider_of_the_deal() {
        let start_epoch = 10;
        let end_epoch = start_epoch + 200 * EPOCHS_IN_DAY;
        let sector_expiry = end_epoch + 100;
        let current_epoch = 5;

        let provider2 = Address::new_id(501);

        let mut rt = setup();
        rt.set_epoch(current_epoch);

        let deal = generate_and_publish_deal(
            &mut rt,
            CLIENT_ADDR,
            &MinerAddresses::default(),
            start_epoch,
            end_epoch,
        );
        activate_deals(&mut rt, sector_expiry, PROVIDER_ADDR, current_epoch, &[deal]);

        // XXX: Difference between go messages: 't0501' has turned into 'f0501'.
        let ret = terminate_deals_raw(&mut rt, provider2, &[deal]);
        expect_abort_contains_message(
            ExitCode::USR_ILLEGAL_STATE,
            "caller f0501 is not the provider f0102 of deal 0",
            ret,
        );
    }

    // Converted from: https://github.com/filecoin-project/specs-actors/blob/master/actors/builtin/market/market_test.go#L1468
    #[test]
    fn fail_when_deal_has_been_published_but_not_activated() {
        let start_epoch = 10;
        let end_epoch = start_epoch + 200 * EPOCHS_IN_DAY;
        let current_epoch = 5;

        let mut rt = setup();
        rt.set_epoch(current_epoch);

        let deal = generate_and_publish_deal(
            &mut rt,
            CLIENT_ADDR,
            &MinerAddresses::default(),
            start_epoch,
            end_epoch,
        );

        let ret = terminate_deals_raw(&mut rt, PROVIDER_ADDR, &[deal]);
        expect_abort_contains_message(ExitCode::USR_ILLEGAL_ARGUMENT, "no state for deal", ret);
        rt.verify();
    }

    // Converted from: https://github.com/filecoin-project/specs-actors/blob/master/actors/builtin/market/market_test.go#L1485
    #[test]
    fn termination_of_all_deals_should_fail_when_one_deal_fails() {
        let start_epoch = 10;
        let end_epoch = start_epoch + 200 * EPOCHS_IN_DAY;
        let sector_expiry = end_epoch + 100;
        let current_epoch = 5;

        let mut rt = setup();
        rt.set_epoch(current_epoch);

        // deal1 would terminate but deal2 will fail because deal2 has not been activated
        let deal1 = generate_and_publish_deal(
            &mut rt,
            CLIENT_ADDR,
            &MinerAddresses::default(),
            start_epoch,
            end_epoch,
        );
        activate_deals(&mut rt, sector_expiry, PROVIDER_ADDR, current_epoch, &[deal1]);
        let deal2 = generate_and_publish_deal(
            &mut rt,
            CLIENT_ADDR,
            &MinerAddresses::default(),
            start_epoch,
            end_epoch + 1,
        );

        let ret = terminate_deals_raw(&mut rt, PROVIDER_ADDR, &[deal1, deal2]);
        expect_abort_contains_message(ExitCode::USR_ILLEGAL_ARGUMENT, "no state for deal", ret);
        rt.verify();

        // verify deal1 has not been terminated
        assert_deals_not_terminated(&mut rt, &[deal1]);
    }
}
