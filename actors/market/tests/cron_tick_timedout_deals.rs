// Copyright 2019-2022 ChainSafe Systems
// SPDX-License-Identifier: Apache-2.0, MIT

use std::convert::TryInto;

use fil_actor_market::balance_table::BALANCE_TABLE_BITWIDTH;
use fil_actor_market::ext::verifreg::RestoreBytesParams;
use fil_actor_market::policy::deal_provider_collateral_bounds;
use fil_actor_market::{
    ext, ActivateDealsParams, Actor as MarketActor, ClientDealProposal, DealMetaArray,
    DealProposal, Label, Method, OnMinerSectorsTerminateParams, PublishStorageDealsParams,
    PublishStorageDealsReturn, State, WithdrawBalanceParams, PROPOSALS_AMT_BITWIDTH,
    STATES_AMT_BITWIDTH,
};
use fil_actor_verifreg::UseBytesParams;
use fil_actors_runtime::cbor::deserialize;
use fil_actors_runtime::network::EPOCHS_IN_DAY;
use fil_actors_runtime::runtime::Policy;
use fil_actors_runtime::test_utils::*;
use fil_actors_runtime::{
    make_empty_map, ActorError, SetMultimap, BURNT_FUNDS_ACTOR_ADDR, SYSTEM_ACTOR_ADDR,
    VERIFIED_REGISTRY_ACTOR_ADDR,
};
use fvm_ipld_amt::Amt;
use fvm_ipld_encoding::{to_vec, RawBytes};
use fvm_shared::address::Address;
use fvm_shared::bigint::bigint_ser::BigIntDe;
use fvm_shared::bigint::BigInt;
use fvm_shared::clock::{ChainEpoch, EPOCH_UNDEFINED};
use fvm_shared::crypto::signature::Signature;
use fvm_shared::deal::DealID;
use fvm_shared::econ::TokenAmount;
use fvm_shared::error::ExitCode;
use fvm_shared::piece::PaddedPieceSize;
use fvm_shared::sector::StoragePower;
use fvm_shared::{HAMT_BIT_WIDTH, METHOD_CONSTRUCTOR, METHOD_SEND, TOTAL_FILECOIN};

use anyhow::anyhow;
use cid::Cid;
use num_traits::{FromPrimitive, Zero};

mod harness;
use harness::*;

const START_EPOCH: ChainEpoch = 50;
const END_EPOCH: ChainEpoch = START_EPOCH + 200 * EPOCHS_IN_DAY;

#[test]
#[ignore = "XXX: Diverges from golang actor"]
fn timed_out_deal_is_slashed_and_deleted() {
    let mut rt = setup();
    let deal_id = generate_and_publish_deal(
        &mut rt,
        CLIENT_ADDR,
        &MinerAddresses::default(),
        START_EPOCH,
        END_EPOCH,
    );
    let deal_proposal = get_deal_proposal(&mut rt, deal_id);

    let c_escrow = get_escrow_balance(&rt, &CLIENT_ADDR).unwrap();

    // do a cron tick for it -> should time out and get slashed
    rt.set_epoch(process_epoch(START_EPOCH, deal_id));
    rt.expect_send(
        *BURNT_FUNDS_ACTOR_ADDR,
        METHOD_SEND,
        RawBytes::default(),
        deal_proposal.provider_collateral.clone(),
        RawBytes::default(),
        ExitCode::OK,
    );
    cron_tick(&mut rt);

    assert_eq!(c_escrow, get_escrow_balance(&rt, &CLIENT_ADDR).unwrap());
    assert!(get_locked_balance(&mut rt, CLIENT_ADDR).is_zero());
    assert_account_zero(&mut rt, PROVIDER_ADDR);
    assert_deal_deleted(&mut rt, deal_id, deal_proposal);
    check_state(&rt);
}

#[test]
#[ignore = "XXX: Diverges from golang actor"]
fn publishing_timed_out_deal_again_should_work_after_cron_tick_as_it_should_no_longer_be_pending() {
    const START_EPOCH: ChainEpoch = 0;
    let mut rt = setup();
    let deal_id = generate_and_publish_deal(
        &mut rt,
        CLIENT_ADDR,
        &MinerAddresses::default(),
        START_EPOCH,
        END_EPOCH,
    );
    let deal_proposal = get_deal_proposal(&mut rt, deal_id);

    // publishing will fail as it will be in pending
    let deal_proposal2 = generate_deal_and_add_funds(
        &mut rt,
        CLIENT_ADDR,
        &MinerAddresses::default(),
        START_EPOCH,
        END_EPOCH,
    );
    let buf = RawBytes::serialize(deal_proposal2.clone()).expect("failed to marshal deal proposal");
    let sig = Signature::new_bls("does not matter".as_bytes().to_vec());
    let client_deal_proposal =
        ClientDealProposal { proposal: deal_proposal2.clone(), client_signature: sig.clone() };
    let params = PublishStorageDealsParams { deals: vec![client_deal_proposal] };
    rt.expect_validate_caller_type(vec![*ACCOUNT_ACTOR_CODE_ID, *MULTISIG_ACTOR_CODE_ID]);
    expect_provider_control_address(&mut rt, PROVIDER_ADDR, OWNER_ADDR, WORKER_ADDR);
    expect_query_network_info(&mut rt);
    rt.set_caller(*ACCOUNT_ACTOR_CODE_ID, WORKER_ADDR);
    rt.expect_verify_signature(ExpectedVerifySig {
        sig,
        signer: deal_proposal2.client,
        plaintext: buf.to_vec(),
        result: Ok(()),
    });
    expect_abort(
        ExitCode::USR_ILLEGAL_ARGUMENT,
        rt.call::<MarketActor>(
            Method::PublishStorageDeals as u64,
            &RawBytes::serialize(params).unwrap(),
        ),
    );
    rt.verify();

    // do a cron tick for it -> should time out and get slashed
    rt.set_epoch(process_epoch(START_EPOCH, deal_id));
    rt.expect_send(
        *BURNT_FUNDS_ACTOR_ADDR,
        METHOD_SEND,
        RawBytes::default(),
        deal_proposal.provider_collateral.clone(),
        RawBytes::default(),
        ExitCode::OK,
    );
    cron_tick(&mut rt);
    assert_deal_deleted(&mut rt, deal_id, deal_proposal);

    // now publishing should work
    generate_and_publish_deal(
        &mut rt,
        CLIENT_ADDR,
        &MinerAddresses::default(),
        START_EPOCH,
        END_EPOCH,
    );
    check_state(&rt);
}

#[test]
#[ignore = "XXX: Diverges from golang actor"]
fn timed_out_and_verified_deals_are_slashed_deleted_and_sent_to_the_registry_actor() {
    let mut rt = setup();
    let mut deal1 = generate_deal_and_add_funds(
        &mut rt,
        CLIENT_ADDR,
        &MinerAddresses::default(),
        START_EPOCH,
        END_EPOCH,
    );
    deal1.verified_deal = true;
    let mut deal2 = generate_deal_and_add_funds(
        &mut rt,
        CLIENT_ADDR,
        &MinerAddresses::default(),
        START_EPOCH,
        END_EPOCH + 1,
    );
    deal2.verified_deal = true;

    // deal3 is NOT verified
    let mut deal3 = generate_deal_and_add_funds(
        &mut rt,
        CLIENT_ADDR,
        &MinerAddresses::default(),
        START_EPOCH,
        END_EPOCH + 2,
    );

    //  publishing verified deals
    rt.set_caller(*ACCOUNT_ACTOR_CODE_ID, WORKER_ADDR);
    let deal_ids = publish_deals(
        &mut rt,
        &MinerAddresses::default(),
        &[deal1.clone(), deal2.clone(), deal3.clone()],
    );

    // do a cron tick for it -> all should time out and get slashed
    // ONLY deal1 and deal2 should be sent to the Registry actor
    rt.set_epoch(process_epoch(START_EPOCH, *deal_ids.last().unwrap()));

    // expected sends to the registry actor
    let param1 = RestoreBytesParams {
        address: deal1.client,
        deal_size: StoragePower::from(deal1.piece_size.0),
    };
    let param2 = RestoreBytesParams {
        address: deal2.client,
        deal_size: StoragePower::from(deal2.piece_size.0),
    };

    rt.expect_send(
        *VERIFIED_REGISTRY_ACTOR_ADDR,
        ext::verifreg::RESTORE_BYTES_METHOD as u64,
        RawBytes::serialize(param1).unwrap(),
        TokenAmount::zero(),
        RawBytes::default(),
        ExitCode::OK,
    );
    rt.expect_send(
        *VERIFIED_REGISTRY_ACTOR_ADDR,
        ext::verifreg::RESTORE_BYTES_METHOD as u64,
        RawBytes::serialize(param2).unwrap(),
        TokenAmount::zero(),
        RawBytes::default(),
        ExitCode::OK,
    );

    let expected_burn = 3 * &deal1.provider_collateral;
    rt.expect_send(
        *BURNT_FUNDS_ACTOR_ADDR,
        METHOD_SEND,
        RawBytes::default(),
        expected_burn,
        RawBytes::default(),
        ExitCode::OK,
    );
    cron_tick(&mut rt);

    // a second cron tick for the same epoch should not change anything
    cron_tick_no_change(&mut rt, CLIENT_ADDR, PROVIDER_ADDR);

    assert_account_zero(&mut rt, PROVIDER_ADDR);
    assert_deal_deleted(&mut rt, deal_ids[0], deal1);
    assert_deal_deleted(&mut rt, deal_ids[1], deal2);
    assert_deal_deleted(&mut rt, deal_ids[2], deal3);
    check_state(&rt);
}
