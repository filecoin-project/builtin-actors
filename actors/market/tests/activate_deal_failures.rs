// Copyright 2019-2022 ChainSafe Systems
// SPDX-License-Identifier: Apache-2.0, MIT

use fvm_shared::address::Address;
use fvm_shared::clock::ChainEpoch;
use fvm_shared::deal::DealID;
use fvm_shared::econ::TokenAmount;
use fvm_shared::error::ExitCode;
use fvm_shared::sector::{RegisteredSealProof, SectorNumber};
use fvm_shared::METHOD_SEND;
use num_traits::Zero;

use fil_actor_market::ext::miner::{PieceReturn, SectorChanges};
use fil_actor_market::BatchActivateDealsResult;
use fil_actor_market::{DealProposal, SectorDeals, EX_DEAL_EXPIRED, NO_ALLOCATION_ID};
use fil_actors_runtime::network::EPOCHS_IN_DAY;
use fil_actors_runtime::test_utils::*;
use fil_actors_runtime::BURNT_FUNDS_ACTOR_ADDR;
use harness::*;

mod harness;

#[test]
fn reject_caller_not_provider() {
    let start_epoch = 10;
    let end_epoch = start_epoch + 200 * EPOCHS_IN_DAY;
    let sector_expiry = end_epoch + 100;

    let rt = setup();
    let provider2_addr = Address::new_id(201);
    let addrs = MinerAddresses { provider: provider2_addr, ..MinerAddresses::default() };
    let deal = generate_deal_and_add_funds(&rt, CLIENT_ADDR, &addrs, start_epoch, end_epoch);
    rt.set_caller(*ACCOUNT_ACTOR_CODE_ID, addrs.worker);
    let deal_id =
        publish_deals(&rt, &addrs, &[deal.clone()], TokenAmount::zero(), NO_ALLOCATION_ID)[0];

    assert_activation_failure(&rt, deal_id, &deal, 1, sector_expiry, ExitCode::USR_FORBIDDEN);
    rt.verify();
    check_state(&rt);
}

#[test]
fn reject_unknown_deal() {
    let start_epoch = 10;
    let end_epoch = start_epoch + 200 * EPOCHS_IN_DAY;
    let sector_expiry = end_epoch + 100;

    let rt = setup();
    let deal = generate_deal_proposal(CLIENT_ADDR, PROVIDER_ADDR, start_epoch, end_epoch);
    assert_activation_failure(&rt, 1234, &deal, 1, sector_expiry, ExitCode::USR_NOT_FOUND);
    rt.verify();
    check_state(&rt);
}

#[test]
fn reject_deal_already_active() {
    let start_epoch = 10;
    let end_epoch = start_epoch + 200 * EPOCHS_IN_DAY;
    let sector_expiry = end_epoch + 100;

    let rt = setup();
    let addrs = MinerAddresses::default();
    let deal = generate_deal_and_add_funds(&rt, CLIENT_ADDR, &addrs, start_epoch, end_epoch);
    rt.set_caller(*ACCOUNT_ACTOR_CODE_ID, addrs.worker);
    let deal_id =
        publish_deals(&rt, &addrs, &[deal.clone()], TokenAmount::zero(), NO_ALLOCATION_ID)[0];
    let sno = 7;
    activate_deals(&rt, sector_expiry, PROVIDER_ADDR, 0, sno, &[deal_id]);

    assert_activation_failure(
        &rt,
        deal_id,
        &deal,
        sno,
        sector_expiry,
        ExitCode::USR_ILLEGAL_ARGUMENT,
    );

    rt.verify();
    check_state(&rt);
}

#[test]
fn reject_proposal_expired() {
    let start_epoch = 10;
    let end_epoch = start_epoch + 200 * EPOCHS_IN_DAY;
    let sector_expiry = end_epoch + 100;

    let rt = setup();
    let addrs = MinerAddresses::default();
    let deal = generate_deal_and_add_funds(&rt, CLIENT_ADDR, &addrs, start_epoch, end_epoch);
    rt.set_caller(*ACCOUNT_ACTOR_CODE_ID, addrs.worker);
    let deal_id =
        publish_deals(&rt, &addrs, &[deal.clone()], TokenAmount::zero(), NO_ALLOCATION_ID)[0];

    let current = end_epoch + 25;
    rt.set_epoch(current);
    assert_activation_failure(&rt, deal_id, &deal, 1, sector_expiry, EX_DEAL_EXPIRED);

    // Show the same behaviour after the deal is cleaned up from state.
    rt.expect_send_simple(
        BURNT_FUNDS_ACTOR_ADDR,
        METHOD_SEND,
        None,
        deal.provider_collateral.clone(),
        None,
        ExitCode::OK,
    );
    cron_tick(&rt);
    assert_deal_deleted(&rt, deal_id, &deal, 0, true);

    assert_activation_failure(&rt, deal_id, &deal, 1, sector_expiry, EX_DEAL_EXPIRED);
}

// Verifies that a deal cannot be activated via either BatchActivateDeals or SectorContentChanged.
fn assert_activation_failure(
    rt: &MockRuntime,
    deal_id: DealID,
    deal: &DealProposal,
    sector_number: SectorNumber,
    sector_expiry: ChainEpoch,
    exit: ExitCode,
) {
    let res = batch_activate_deals_raw(
        rt,
        PROVIDER_ADDR,
        vec![SectorDeals {
            sector_number,
            sector_expiry,
            sector_type: RegisteredSealProof::StackedDRG8MiBV1,
            deal_ids: vec![deal_id],
        }],
        false,
        &[],
    )
    .unwrap();
    let res: BatchActivateDealsResult =
        res.unwrap().deserialize().expect("BatchActivateDealsResult failed to deserialize");
    assert_eq!(res.activation_results.codes(), vec![exit]);

    let piece = piece_info_from_deal(deal_id, deal);
    let changes = vec![SectorChanges {
        sector: 1,
        minimum_commitment_epoch: sector_expiry,
        added: vec![piece],
    }];
    let ret = sector_content_changed(rt, PROVIDER_ADDR, changes).unwrap();
    assert_eq!(vec![PieceReturn { accepted: false }], ret.sectors[0].added);
}
