// Copyright 2019-2022 ChainSafe Systems
// SPDX-License-Identifier: Apache-2.0, MIT

use fil_actor_market::policy::deal_provider_collateral_bounds;
use fil_actor_market::{
    Actor as MarketActor, ClientDealProposal, DealProposal, MarketNotifyDealParams, Method,
    PublishStorageDealsParams, PublishStorageDealsReturn, State, MARKET_NOTIFY_DEAL_METHOD,
};
use fil_actors_runtime::network::EPOCHS_IN_DAY;
use fil_actors_runtime::runtime::Policy;
use fil_actors_runtime::test_utils::*;
use fvm_ipld_encoding::RawBytes;
use fvm_shared::address::Address;
use fvm_shared::bigint::BigInt;
use fvm_shared::clock::ChainEpoch;
use fvm_shared::crypto::signature::Signature;
use fvm_shared::econ::TokenAmount;
use fvm_shared::error::ExitCode;
use fvm_shared::piece::PaddedPieceSize;

use fvm_shared::TOTAL_FILECOIN;

use cid::Cid;
use fil_actor_market::ext::account::{AuthenticateMessageParams, AUTHENTICATE_MESSAGE_METHOD};

mod harness;

use fvm_ipld_encoding::ipld_block::IpldBlock;
use harness::*;
use num_traits::Zero;

#[test]
fn deal_end_after_deal_start() {
    let f = |_rt: &mut MockRuntime, d: &mut DealProposal| {
        d.start_epoch = 10;
        d.end_epoch = 9;
    };
    assert_deal_failure(true, f, ExitCode::USR_ILLEGAL_ARGUMENT, true);
}

#[test]
fn current_epoch_greater_than_start_epoch() {
    let f = |rt: &mut MockRuntime, d: &mut DealProposal| {
        d.start_epoch = rt.epoch - 1;
    };
    assert_deal_failure(true, f, ExitCode::USR_ILLEGAL_ARGUMENT, true);
}

#[test]
fn deal_duration_greater_than_max_deal_duration() {
    let f = |_rt: &mut MockRuntime, d: &mut DealProposal| {
        d.start_epoch = ChainEpoch::from(10);
        d.end_epoch = d.start_epoch + (540 * EPOCHS_IN_DAY) + 1
    };
    assert_deal_failure(true, f, ExitCode::USR_ILLEGAL_ARGUMENT, true);
}

#[test]
fn negative_price_per_epoch() {
    let f = |_rt: &mut MockRuntime, d: &mut DealProposal| {
        d.storage_price_per_epoch = TokenAmount::from_atto(-1);
    };
    assert_deal_failure(true, f, ExitCode::USR_ILLEGAL_ARGUMENT, true);
}

#[test]
fn price_per_epoch_greater_than_total_filecoin() {
    let f = |_rt: &mut MockRuntime, d: &mut DealProposal| {
        d.storage_price_per_epoch = &*TOTAL_FILECOIN + TokenAmount::from_atto(1);
    };
    assert_deal_failure(true, f, ExitCode::USR_ILLEGAL_ARGUMENT, true);
}

#[test]
fn negative_provider_collateral() {
    let f = |_rt: &mut MockRuntime, d: &mut DealProposal| {
        d.provider_collateral = TokenAmount::from_atto(-1);
    };
    assert_deal_failure(true, f, ExitCode::USR_ILLEGAL_ARGUMENT, true);
}

#[test]
fn provider_collateral_greater_than_max_collateral() {
    let f = |_rt: &mut MockRuntime, d: &mut DealProposal| {
        d.provider_collateral = &*TOTAL_FILECOIN + TokenAmount::from_atto(1);
    };
    assert_deal_failure(true, f, ExitCode::USR_ILLEGAL_ARGUMENT, true);
}

#[test]
fn provider_collateral_less_than_bound() {
    let f = |_rt: &mut MockRuntime, d: &mut DealProposal| {
        let circ_supply = TokenAmount::from_atto(1i64 << 50);
        let (provider_min, _) = deal_provider_collateral_bounds(
            &Policy::default(),
            PaddedPieceSize(2048),
            &BigInt::from(0u8),
            &BigInt::from(0u8),
            &circ_supply,
        );
        d.provider_collateral = provider_min - TokenAmount::from_atto(1);
    };
    assert_deal_failure(true, f, ExitCode::USR_ILLEGAL_ARGUMENT, true);
}

#[test]
fn negative_client_collateral() {
    let f = |_rt: &mut MockRuntime, d: &mut DealProposal| {
        d.client_collateral = TokenAmount::from_atto(-1);
    };
    assert_deal_failure(true, f, ExitCode::USR_ILLEGAL_ARGUMENT, true);
}

#[test]
fn client_collateral_greater_than_max_collateral() {
    let f = |_rt: &mut MockRuntime, d: &mut DealProposal| {
        d.client_collateral = &*TOTAL_FILECOIN + TokenAmount::from_atto(1);
    };
    assert_deal_failure(true, f, ExitCode::USR_ILLEGAL_ARGUMENT, true);
}

#[test]
fn client_does_not_have_enough_balance_for_collateral() {
    let f = |rt: &mut MockRuntime, d: &mut DealProposal| {
        add_participant_funds(
            rt,
            CLIENT_ADDR,
            d.client_balance_requirement() - TokenAmount::from_atto(1),
        );
        add_provider_funds(rt, d.provider_collateral.clone(), &MinerAddresses::default());
    };
    assert_deal_failure(false, f, ExitCode::USR_ILLEGAL_ARGUMENT, true);
}

#[test]
fn provider_does_not_have_enough_balance_for_collateral() {
    let f = |rt: &mut MockRuntime, d: &mut DealProposal| {
        add_participant_funds(rt, CLIENT_ADDR, d.client_balance_requirement());
        add_provider_funds(
            rt,
            d.provider_collateral.clone() - TokenAmount::from_atto(1),
            &MinerAddresses::default(),
        );
    };
    assert_deal_failure(false, f, ExitCode::USR_ILLEGAL_ARGUMENT, true);
}

#[test]
fn client_address_does_not_exist() {
    let f = |_rt: &mut MockRuntime, d: &mut DealProposal| {
        d.client = Address::new_id(1);
    };
    assert_deal_failure(true, f, ExitCode::USR_ILLEGAL_ARGUMENT, true);
}

#[test]
fn unable_to_resolve_client_address() {
    let f = |_rt: &mut MockRuntime, d: &mut DealProposal| {
        d.client = new_bls_addr(1);
    };
    assert_deal_failure(true, f, ExitCode::USR_ILLEGAL_ARGUMENT, true);
}

#[test]
fn signature_is_invalid() {
    let f = |_rt: &mut MockRuntime, _d: &mut DealProposal| {};
    assert_deal_failure(true, f, ExitCode::USR_ILLEGAL_ARGUMENT, false);
}

#[test]
fn no_entry_for_client_in_locked_balance_table() {
    let f = |rt: &mut MockRuntime, d: &mut DealProposal| {
        add_provider_funds(rt, d.provider_collateral.clone(), &MinerAddresses::default());
    };
    assert_deal_failure(false, f, ExitCode::USR_ILLEGAL_ARGUMENT, true);
}

#[test]
fn no_entry_for_provider_in_locked_balance_table() {
    let f = |rt: &mut MockRuntime, d: &mut DealProposal| {
        add_participant_funds(rt, CLIENT_ADDR, d.client_balance_requirement());
    };
    assert_deal_failure(false, f, ExitCode::USR_ILLEGAL_ARGUMENT, true);
}

#[test]
fn bad_piece_cid() {
    let f = |_rt: &mut MockRuntime, d: &mut DealProposal| {
        d.piece_cid = Cid::default();
    };
    assert_deal_failure(true, f, ExitCode::USR_ILLEGAL_ARGUMENT, true);
}

#[test]
fn zero_piece_size() {
    let f = |_rt: &mut MockRuntime, d: &mut DealProposal| {
        d.piece_size = PaddedPieceSize(0u64);
    };
    assert_deal_failure(true, f, ExitCode::USR_ILLEGAL_ARGUMENT, true);
}

#[test]
fn piece_size_less_than_128_bytes() {
    let f = |_rt: &mut MockRuntime, d: &mut DealProposal| {
        d.piece_size = PaddedPieceSize(64u64);
    };
    assert_deal_failure(true, f, ExitCode::USR_ILLEGAL_ARGUMENT, true);
}

#[test]
fn piece_size_is_not_a_power_of_2() {
    let f = |_rt: &mut MockRuntime, d: &mut DealProposal| {
        d.piece_size = PaddedPieceSize(254u64);
    };
    assert_deal_failure(true, f, ExitCode::USR_ILLEGAL_ARGUMENT, true);
}

#[test]
fn fail_when_client_has_some_funds_but_not_enough_for_a_deal() {
    let mut rt = setup();

    let amount = TokenAmount::from_atto(100u8);
    add_participant_funds(&mut rt, CLIENT_ADDR, amount.clone());
    let start_epoch = 42;
    let end_epoch = start_epoch + 200 * EPOCHS_IN_DAY;
    let deal1 = generate_deal_proposal(CLIENT_ADDR, PROVIDER_ADDR, start_epoch, end_epoch);
    assert!(amount < deal1.client_balance_requirement());
    add_provider_funds(&mut rt, deal1.clone().provider_collateral, &MinerAddresses::default());
    publish_deals_expect_abort(
        &mut rt,
        &MinerAddresses::default(),
        deal1,
        ExitCode::USR_ILLEGAL_ARGUMENT,
    );

    check_state(&rt);
}

#[test]
fn fail_when_provider_has_some_funds_but_not_enough_for_a_deal() {
    let start_epoch = 10;
    let end_epoch = start_epoch + 200 * EPOCHS_IN_DAY;

    let mut rt = setup();

    let amount = TokenAmount::from_atto(1u8);
    add_provider_funds(&mut rt, amount.clone(), &MinerAddresses::default());
    let deal1 = generate_deal_proposal(CLIENT_ADDR, PROVIDER_ADDR, start_epoch, end_epoch);
    assert!(amount < deal1.client_balance_requirement());
    add_participant_funds(&mut rt, CLIENT_ADDR, deal1.client_balance_requirement());

    let buf = RawBytes::serialize(deal1.clone()).expect("failed to marshal deal proposal");
    let sig = Signature::new_bls(buf.to_vec());
    let params = PublishStorageDealsParams {
        deals: vec![ClientDealProposal { proposal: deal1.clone(), client_signature: sig }],
    };

    rt.expect_validate_caller_any();
    expect_provider_is_control_address(&mut rt, PROVIDER_ADDR, WORKER_ADDR, true);
    expect_query_network_info(&mut rt);
    rt.set_caller(*ACCOUNT_ACTOR_CODE_ID, WORKER_ADDR);

    let auth_param = IpldBlock::serialize_cbor(&AuthenticateMessageParams {
        signature: buf.to_vec(),
        message: buf.to_vec(),
    })
    .unwrap();

    rt.expect_send(
        deal1.client,
        AUTHENTICATE_MESSAGE_METHOD,
        auth_param,
        TokenAmount::zero(),
        None,
        ExitCode::OK,
    );

    expect_abort(
        ExitCode::USR_ILLEGAL_ARGUMENT,
        rt.call::<MarketActor>(
            Method::PublishStorageDeals as u64,
            IpldBlock::serialize_cbor(&params).unwrap(),
        ),
    );

    rt.verify();
    check_state(&rt);
}

#[test]
fn fail_when_deals_have_different_providers() {
    let start_epoch = 10;
    let end_epoch = start_epoch + 200 * EPOCHS_IN_DAY;

    let mut rt = setup();
    let st: State = rt.get_state();
    let next_deal_id = st.next_id;

    let deal1 = generate_deal_and_add_funds(
        &mut rt,
        CLIENT_ADDR,
        &MinerAddresses::default(),
        start_epoch,
        end_epoch,
    );
    let m2 = MinerAddresses { provider: Address::new_id(1000), ..MinerAddresses::default() };

    let deal2 = generate_deal_and_add_funds(&mut rt, CLIENT_ADDR, &m2, 1, end_epoch);

    let buf1 = RawBytes::serialize(deal1.clone()).expect("failed to marshal deal proposal");
    let buf2 = RawBytes::serialize(deal2.clone()).expect("failed to marshal deal proposal");
    let sig1 = Signature::new_bls(buf1.to_vec());
    let sig2 = Signature::new_bls(buf2.to_vec());
    let params = PublishStorageDealsParams {
        deals: vec![
            ClientDealProposal { proposal: deal1.clone(), client_signature: sig1 },
            ClientDealProposal { proposal: deal2.clone(), client_signature: sig2 },
        ],
    };

    rt.expect_validate_caller_any();
    expect_provider_is_control_address(&mut rt, PROVIDER_ADDR, WORKER_ADDR, true);
    expect_query_network_info(&mut rt);
    rt.set_caller(*ACCOUNT_ACTOR_CODE_ID, WORKER_ADDR);
    let authenticate_param1 = IpldBlock::serialize_cbor(&AuthenticateMessageParams {
        signature: buf1.to_vec(),
        message: buf1.to_vec(),
    })
    .unwrap();
    let authenticate_param2 = IpldBlock::serialize_cbor(&AuthenticateMessageParams {
        signature: buf2.to_vec(),
        message: buf2.to_vec(),
    })
    .unwrap();

    rt.expect_send(
        deal1.client,
        AUTHENTICATE_MESSAGE_METHOD as u64,
        authenticate_param1,
        TokenAmount::zero(),
        None,
        ExitCode::OK,
    );
    rt.expect_send(
        deal2.client,
        AUTHENTICATE_MESSAGE_METHOD as u64,
        authenticate_param2,
        TokenAmount::zero(),
        None,
        ExitCode::OK,
    );

    // only valid deals are notified
    let notify_param1 = IpldBlock::serialize_cbor(&MarketNotifyDealParams {
        proposal: RawBytes::serialize(&deal1).expect("failed to marshal deal proposal").to_vec(),
        deal_id: next_deal_id,
    })
    .unwrap();

    rt.expect_send(
        deal1.client,
        MARKET_NOTIFY_DEAL_METHOD,
        notify_param1,
        TokenAmount::zero(),
        None,
        ExitCode::USR_UNHANDLED_MESSAGE,
    );

    let psd_ret: PublishStorageDealsReturn = rt
        .call::<MarketActor>(
            Method::PublishStorageDeals as u64,
            IpldBlock::serialize_cbor(&params).unwrap(),
        )
        .unwrap()
        .unwrap()
        .deserialize()
        .unwrap();

    let valid: Vec<u64> = psd_ret.valid_deals.bounded_iter(std::u64::MAX).unwrap().collect();
    assert_eq!(vec![0], valid);

    rt.verify();
    check_state(&rt);
}

#[test]
fn fail_when_no_deals_in_params() {
    let mut rt = setup();
    let params = PublishStorageDealsParams { deals: vec![] };
    rt.set_caller(*ACCOUNT_ACTOR_CODE_ID, WORKER_ADDR);
    rt.expect_validate_caller_any();
    expect_abort(
        ExitCode::USR_ILLEGAL_ARGUMENT,
        rt.call::<MarketActor>(
            Method::PublishStorageDeals as u64,
            IpldBlock::serialize_cbor(&params).unwrap(),
        ),
    );
    check_state(&rt);
}

#[test]
fn fail_to_resolve_provider_address() {
    let start_epoch = 10;
    let end_epoch = start_epoch + 200 * EPOCHS_IN_DAY;

    let mut rt = setup();
    let mut deal = generate_deal_proposal(CLIENT_ADDR, PROVIDER_ADDR, start_epoch, end_epoch);
    deal.provider = new_bls_addr(100);

    let sig = Signature::new_bls("does not matter".as_bytes().to_vec());
    let params = PublishStorageDealsParams {
        deals: vec![ClientDealProposal { proposal: deal, client_signature: sig }],
    };
    rt.set_caller(*ACCOUNT_ACTOR_CODE_ID, WORKER_ADDR);
    rt.expect_validate_caller_any();
    expect_abort(
        ExitCode::USR_NOT_FOUND,
        rt.call::<MarketActor>(
            Method::PublishStorageDeals as u64,
            IpldBlock::serialize_cbor(&params).unwrap(),
        ),
    );
    check_state(&rt);
}

#[test]
fn caller_is_not_the_same_as_the_worker_address_for_miner() {
    let start_epoch = 10;
    let end_epoch = start_epoch + 200 * EPOCHS_IN_DAY;

    let mut rt = setup();
    let deal = generate_deal_proposal(CLIENT_ADDR, PROVIDER_ADDR, start_epoch, end_epoch);
    let sig = Signature::new_bls("does not matter".as_bytes().to_vec());
    let params = PublishStorageDealsParams {
        deals: vec![ClientDealProposal { proposal: deal, client_signature: sig }],
    };

    rt.expect_validate_caller_any();
    expect_provider_is_control_address(&mut rt, PROVIDER_ADDR, Address::new_id(999), false);
    rt.set_caller(*ACCOUNT_ACTOR_CODE_ID, Address::new_id(999));
    expect_abort(
        ExitCode::USR_FORBIDDEN,
        rt.call::<MarketActor>(
            Method::PublishStorageDeals as u64,
            IpldBlock::serialize_cbor(&params).unwrap(),
        ),
    );

    rt.verify();
    check_state(&rt);
}

#[test]
fn fails_if_provider_is_not_a_storage_miner_actor() {
    let mut rt = setup();

    // deal provider will be a Storage Miner Actor.
    let p2 = Address::new_id(505);
    rt.set_address_actor_type(p2, *POWER_ACTOR_CODE_ID);
    let deal = generate_deal_proposal(CLIENT_ADDR, p2, ChainEpoch::from(1), ChainEpoch::from(5));

    let sig = Signature::new_bls("does not matter".as_bytes().to_vec());
    let params = PublishStorageDealsParams {
        deals: vec![ClientDealProposal { proposal: deal, client_signature: sig }],
    };

    rt.expect_validate_caller_any();
    rt.set_caller(*ACCOUNT_ACTOR_CODE_ID, WORKER_ADDR);
    expect_abort(
        ExitCode::USR_ILLEGAL_ARGUMENT,
        rt.call::<MarketActor>(
            Method::PublishStorageDeals as u64,
            IpldBlock::serialize_cbor(&params).unwrap(),
        ),
    );

    rt.verify();
    check_state(&rt);
}
