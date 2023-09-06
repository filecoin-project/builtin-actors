// Copyright 2019-2022 ChainSafe Systems
// SPDX-License-Identifier: Apache-2.0, MIT
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
    activate_deals_legacy(&rt, sector_expiry, PROVIDER_ADDR, current_epoch, &[deal1, deal2, deal3]);

    let addrs = MinerAddresses { provider: provider2, ..MinerAddresses::default() };
    let deal4 = generate_and_publish_deal(&rt, CLIENT_ADDR, &addrs, start_epoch, end_epoch);
    let deal5 = generate_and_publish_deal(&rt, CLIENT_ADDR, &addrs, start_epoch, end_epoch + 1);
    activate_deals_legacy(&rt, sector_expiry, provider2, current_epoch, &[deal4, deal5]);

    let prop1 = get_deal_proposal(&rt, deal1);
    terminate_deals_and_assert_balances(&rt, CLIENT_ADDR, PROVIDER_ADDR, &[deal1]);
    assert_deal_deleted(&rt, deal1, &prop1);
    assert_deals_not_terminated(&rt, &[deal2, deal3, deal4, deal5]);

    let prop5 = get_deal_proposal(&rt, deal5);
    terminate_deals_and_assert_balances(&rt, CLIENT_ADDR, provider2, &[deal5]);
    assert_deal_deleted(&rt, deal5, &prop5);
    assert_deals_not_terminated(&rt, &[deal2, deal3, deal4]);

    let prop2 = get_deal_proposal(&rt, deal2);
    let prop3 = get_deal_proposal(&rt, deal3);
    terminate_deals_and_assert_balances(&rt, CLIENT_ADDR, PROVIDER_ADDR, &[deal2, deal3]);
    assert_deal_deleted(&rt, deal2, &prop2);
    assert_deal_deleted(&rt, deal3, &prop3);
    assert_deals_not_terminated(&rt, &[deal4]);

    let prop4 = get_deal_proposal(&rt, deal4);
    terminate_deals_and_assert_balances(&rt, CLIENT_ADDR, provider2, &[deal4]);
    assert_deal_deleted(&rt, deal4, &prop4);
    check_state(&rt);
}

#[test]
fn ignore_deal_proposal_that_does_not_exist() {
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
    activate_deals_legacy(&rt, sector_expiry, PROVIDER_ADDR, current_epoch, &[deal1, deal2, deal3]);

    let new_epoch = end_epoch - 1;
    rt.set_epoch(new_epoch);

    let prop1 = get_deal_proposal(&rt, deal1);
    let prop2 = get_deal_proposal(&rt, deal2);

    terminate_deals_and_assert_balances(&rt, CLIENT_ADDR, PROVIDER_ADDR, &[deal1, deal2, deal3]);
    assert_deal_deleted(&rt, deal1, &prop1);
    assert_deal_deleted(&rt, deal2, &prop2);
    assert_deals_not_terminated(&rt, &[deal3]);
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
    activate_deals_legacy(&rt, sector_expiry, PROVIDER_ADDR, current_epoch, &[deal1, deal2, deal3]);

    let new_epoch = end_epoch - 1;
    rt.set_epoch(new_epoch);

    let prop1 = get_deal_proposal(&rt, deal1);
    let prop2 = get_deal_proposal(&rt, deal2);

    terminate_deals_and_assert_balances(&rt, CLIENT_ADDR, PROVIDER_ADDR, &[deal1, deal2, deal3]);
    assert_deal_deleted(&rt, deal1, &prop1);
    assert_deal_deleted(&rt, deal2, &prop2);
    assert_deals_not_terminated(&rt, &[deal3]);
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
    activate_deals_legacy(&rt, sector_expiry, PROVIDER_ADDR, current_epoch, &deal_ids);

    let new_epoch = end_epoch - 1;
    rt.set_epoch(new_epoch);
    cron_tick(&rt);
    // expired deal deleted normally
    assert_deal_deleted(&rt, deal_ids[1], &deal2);
    assert_deals_not_terminated(&rt, &deal_ids[0..0]);

    terminate_deals_and_assert_balances(&rt, CLIENT_ADDR, PROVIDER_ADDR, &deal_ids);
    // terminated deal deleted
    assert_deal_deleted(&rt, deal_ids[0], &deal1);

    // terminated deal has a dangling deal op, normally expired deal doesn't
    check_state(&rt);
}

#[test]
fn terminating_a_deal_the_second_time_does_not_affect_existing_deals_in_the_batch() {
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
    let [deal1, _, _]: [DealID; 3] = deals.as_slice().try_into().unwrap();
    activate_deals_legacy(&rt, sector_expiry, PROVIDER_ADDR, current_epoch, &deals);

    // terminating the deal and check balances update as expected
    terminate_deals_and_assert_balances(&rt, CLIENT_ADDR, PROVIDER_ADDR, &[deal1]);

    // terminating deals included previously terminated and check balances update as expected
    rt.set_epoch(current_epoch + 1);
    terminate_deals_and_assert_balances(&rt, CLIENT_ADDR, PROVIDER_ADDR, &deals);

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
    activate_deals_legacy(&rt, sector_expiry, PROVIDER_ADDR, current_epoch, &[deal1]);
    rt.set_epoch(end_epoch);
    terminate_deals_and_assert_balances(&rt, CLIENT_ADDR, PROVIDER_ADDR, &[deal1]);
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
    activate_deals_legacy(&rt, sector_expiry, PROVIDER_ADDR, current_epoch, &[deal2]);
    rt.set_epoch(end_epoch + 1);
    terminate_deals_and_assert_balances(&rt, CLIENT_ADDR, PROVIDER_ADDR, &[deal2]);
    assert_deals_not_terminated(&rt, &[deal2]);

    check_state(&rt);
}

#[test]
fn fail_when_caller_is_not_a_storage_miner_actor() {
    let rt = setup();
    rt.expect_validate_caller_type(vec![Type::Miner]);
    rt.set_caller(*ACCOUNT_ACTOR_CODE_ID, PROVIDER_ADDR);
    let params = OnMinerSectorsTerminateParams { epoch: *rt.epoch.borrow(), deal_ids: vec![] };

    // XXX: Which exit code is correct: SYS_FORBIDDEN(8) or USR_FORBIDDEN(18)?
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

#[test]
fn fail_when_caller_is_not_the_provider_of_the_deal() {
    let start_epoch = 10;
    let end_epoch = start_epoch + 200 * EPOCHS_IN_DAY;
    let sector_expiry = end_epoch + 100;
    let current_epoch = 5;

    let provider2 = Address::new_id(501);

    let rt = setup();
    rt.set_epoch(current_epoch);

    let deal = generate_and_publish_deal(
        &rt,
        CLIENT_ADDR,
        &MinerAddresses::default(),
        start_epoch,
        end_epoch,
    );
    activate_deals(&rt, sector_expiry, PROVIDER_ADDR, current_epoch, &[deal]);

    // XXX: Difference between go messages: 't0501' has turned into 'f0501'.
    let ret = terminate_deals_raw(&rt, provider2, &[deal]);
    expect_abort_contains_message(
        ExitCode::USR_ILLEGAL_STATE,
        "caller f0501 is not the provider f0102 of deal 0",
        ret,
    );

    check_state(&rt);
}

#[test]
fn fail_when_deal_has_been_published_but_not_activated() {
    let start_epoch = 10;
    let end_epoch = start_epoch + 200 * EPOCHS_IN_DAY;
    let current_epoch = 5;

    let rt = setup();
    rt.set_epoch(current_epoch);

    let deal = generate_and_publish_deal(
        &rt,
        CLIENT_ADDR,
        &MinerAddresses::default(),
        start_epoch,
        end_epoch,
    );

    let ret = terminate_deals_raw(&rt, PROVIDER_ADDR, &[deal]);
    expect_abort_contains_message(ExitCode::USR_ILLEGAL_ARGUMENT, "no state for deal", ret);
    rt.verify();
    check_state(&rt);
}

#[test]
fn termination_of_all_deals_should_fail_when_one_deal_fails() {
    let start_epoch = 10;
    let end_epoch = start_epoch + 200 * EPOCHS_IN_DAY;
    let sector_expiry = end_epoch + 100;
    let current_epoch = 5;

    let rt = setup();
    rt.set_epoch(current_epoch);

    // deal1 would terminate but deal2 will fail because deal2 has not been activated
    let deal1 = generate_and_publish_deal(
        &rt,
        CLIENT_ADDR,
        &MinerAddresses::default(),
        start_epoch,
        end_epoch,
    );
    activate_deals(&rt, sector_expiry, PROVIDER_ADDR, current_epoch, &[deal1]);
    let deal2 = generate_and_publish_deal(
        &rt,
        CLIENT_ADDR,
        &MinerAddresses::default(),
        start_epoch,
        end_epoch + 1,
    );

    let ret = terminate_deals_raw(&rt, PROVIDER_ADDR, &[deal1, deal2]);
    expect_abort_contains_message(ExitCode::USR_ILLEGAL_ARGUMENT, "no state for deal", ret);
    rt.verify();

    // verify deal1 has not been terminated
    assert_deals_not_terminated(&rt, &[deal1]);
    check_state(&rt);
}
