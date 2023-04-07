// Copyright 2019-2022 ChainSafe Systems
// SPDX-License-Identifier: Apache-2.0, MIT

use fil_actor_market::{
    Actor as MarketActor, ClientDealProposal, Method, PublishStorageDealsParams,
};
use fil_actors_runtime::network::EPOCHS_IN_DAY;
use fil_actors_runtime::test_utils::*;
use fil_actors_runtime::BURNT_FUNDS_ACTOR_ADDR;
use fvm_ipld_encoding::RawBytes;
use fvm_shared::clock::ChainEpoch;
use fvm_shared::crypto::signature::Signature;
use fvm_shared::econ::TokenAmount;
use fvm_shared::error::ExitCode;
use fvm_shared::METHOD_SEND;

use fil_actor_market::ext::account::{AuthenticateMessageParams, AUTHENTICATE_MESSAGE_METHOD};
use fvm_ipld_encoding::ipld_block::IpldBlock;
use fvm_shared::sys::SendFlags;
use num_traits::Zero;

mod harness;

use harness::*;

const START_EPOCH: ChainEpoch = 50;
const END_EPOCH: ChainEpoch = START_EPOCH + 200 * EPOCHS_IN_DAY;

#[test]
fn timed_out_deal_is_slashed_and_deleted() {
    let rt = setup();
    let deal_id = generate_and_publish_deal(
        &rt,
        CLIENT_ADDR,
        &MinerAddresses::default(),
        START_EPOCH,
        END_EPOCH,
    );
    let deal_proposal = get_deal_proposal(&rt, deal_id);

    let c_escrow = get_balance(&rt, &CLIENT_ADDR).balance;

    // do a cron tick for it -> should time out and get slashed
    rt.set_epoch(process_epoch(START_EPOCH, deal_id));
    rt.expect_send_simple(
        BURNT_FUNDS_ACTOR_ADDR,
        METHOD_SEND,
        None,
        deal_proposal.provider_collateral.clone(),
        None,
        ExitCode::OK,
    );
    cron_tick(&rt);

    let client_acct = get_balance(&rt, &CLIENT_ADDR);
    assert_eq!(c_escrow, client_acct.balance);
    assert!(client_acct.locked.is_zero());
    assert_account_zero(&rt, PROVIDER_ADDR);
    assert_deal_deleted(&rt, deal_id, deal_proposal);
    check_state(&rt);
}

#[test]
fn publishing_timed_out_deal_again_should_work_after_cron_tick_as_it_should_no_longer_be_pending() {
    const START_EPOCH: ChainEpoch = 0;
    let rt = setup();
    let deal_id = generate_and_publish_deal(
        &rt,
        CLIENT_ADDR,
        &MinerAddresses::default(),
        START_EPOCH,
        END_EPOCH,
    );
    let deal_proposal = get_deal_proposal(&rt, deal_id);

    // publishing will fail as it will be in pending
    let deal_proposal2 = generate_deal_and_add_funds(
        &rt,
        CLIENT_ADDR,
        &MinerAddresses::default(),
        START_EPOCH,
        END_EPOCH,
    );
    let buf = RawBytes::serialize(deal_proposal2.clone()).expect("failed to marshal deal proposal");
    let sig = Signature::new_bls(buf.to_vec());
    let client_deal_proposal =
        ClientDealProposal { proposal: deal_proposal2.clone(), client_signature: sig };
    let params = PublishStorageDealsParams { deals: vec![client_deal_proposal] };
    rt.expect_validate_caller_any();
    expect_provider_is_control_address(&rt, PROVIDER_ADDR, WORKER_ADDR, true);
    expect_query_network_info(&rt);
    rt.set_caller(*ACCOUNT_ACTOR_CODE_ID, WORKER_ADDR);
    let auth_param = IpldBlock::serialize_cbor(&AuthenticateMessageParams {
        signature: buf.to_vec(),
        message: buf.to_vec(),
    })
    .unwrap();

    rt.expect_send(
        deal_proposal2.client,
        AUTHENTICATE_MESSAGE_METHOD,
        auth_param,
        TokenAmount::zero(),
        None,
        SendFlags::READ_ONLY,
        AUTHENTICATE_MESSAGE_RESPONSE.clone(),
        ExitCode::OK,
        None,
    );

    expect_abort(
        ExitCode::USR_ILLEGAL_ARGUMENT,
        rt.call::<MarketActor>(
            Method::PublishStorageDeals as u64,
            IpldBlock::serialize_cbor(&params).unwrap(),
        ),
    );
    rt.verify();

    // do a cron tick for it -> should time out and get slashed
    rt.set_epoch(process_epoch(START_EPOCH, deal_id));
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

    // now publishing should work
    generate_and_publish_deal(&rt, CLIENT_ADDR, &MinerAddresses::default(), START_EPOCH, END_EPOCH);
    check_state(&rt);
}

#[test]
fn timed_out_and_verified_deals_are_slashed_deleted() {
    let rt = setup();
    let mut deal1 = generate_deal_and_add_funds(
        &rt,
        CLIENT_ADDR,
        &MinerAddresses::default(),
        START_EPOCH,
        END_EPOCH,
    );
    deal1.verified_deal = true;
    let mut deal2 = generate_deal_and_add_funds(
        &rt,
        CLIENT_ADDR,
        &MinerAddresses::default(),
        START_EPOCH,
        END_EPOCH + 1,
    );
    deal2.verified_deal = true;

    // deal3 is NOT verified
    let deal3 = generate_deal_and_add_funds(
        &rt,
        CLIENT_ADDR,
        &MinerAddresses::default(),
        START_EPOCH,
        END_EPOCH + 2,
    );

    //  publishing verified deals
    rt.set_caller(*ACCOUNT_ACTOR_CODE_ID, WORKER_ADDR);
    let deal_ids = publish_deals(
        &rt,
        &MinerAddresses::default(),
        &[deal1.clone(), deal2.clone(), deal3.clone()],
        TokenAmount::from_whole(deal1.piece_size.0 * 10),
        1,
    );
    assert_eq!(3, deal_ids.len());

    // do a cron tick for it -> all should time out and get slashed
    // ONLY deal1 and deal2 should be sent to the Registry actor
    rt.set_epoch(process_epoch(START_EPOCH, *deal_ids.last().unwrap()));

    let expected_burn = 3 * &deal1.provider_collateral;
    rt.expect_send_simple(
        BURNT_FUNDS_ACTOR_ADDR,
        METHOD_SEND,
        None,
        expected_burn,
        None,
        ExitCode::OK,
    );
    cron_tick(&rt);

    // a second cron tick for the same epoch should not change anything
    cron_tick_no_change(&rt, CLIENT_ADDR, PROVIDER_ADDR);

    assert_account_zero(&rt, PROVIDER_ADDR);
    assert_deal_deleted(&rt, deal_ids[0], deal1);
    assert_deal_deleted(&rt, deal_ids[1], deal2);
    assert_deal_deleted(&rt, deal_ids[2], deal3);
    check_state(&rt);
}
