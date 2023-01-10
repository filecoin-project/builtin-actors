// Copyright 2019-2022 ChainSafe Systems
// SPDX-License-Identifier: Apache-2.0, MIT

mod harness;

use fil_actor_market::{Actor as MarketActor, Method, SectorDeals, VerifyDealsForActivationParams};
use fil_actors_runtime::runtime::builtins::Type;
use fil_actors_runtime::test_utils::{
    expect_abort, expect_abort_contains_message, make_piece_cid, ACCOUNT_ACTOR_CODE_ID,
    MINER_ACTOR_CODE_ID,
};
use fil_actors_runtime::EPOCHS_IN_DAY;
use fvm_ipld_encoding::ipld_block::IpldBlock;
use fvm_shared::address::Address;
use fvm_shared::bigint::BigInt;
use fvm_shared::clock::ChainEpoch;
use fvm_shared::error::ExitCode;
use fvm_shared::piece::PieceInfo;
use fvm_shared::sector::RegisteredSealProof;
use harness::*;
use num_traits::Zero;

const START_EPOCH: ChainEpoch = 10;
const CURR_EPOCH: ChainEpoch = START_EPOCH;
const END_EPOCH: ChainEpoch = 200 * EPOCHS_IN_DAY;
const SECTOR_EXPIRY: ChainEpoch = END_EPOCH + 200;
const MINER_ADDRESSES: MinerAddresses = MinerAddresses {
    owner: OWNER_ADDR,
    worker: WORKER_ADDR,
    provider: PROVIDER_ADDR,
    control: vec![],
};

#[test]
fn verify_deal_and_activate_to_get_deal_space_for_unverified_deal_proposal() {
    let mut rt = setup();
    let deal_id =
        generate_and_publish_deal(&mut rt, CLIENT_ADDR, &MINER_ADDRESSES, START_EPOCH, END_EPOCH);
    let deal_proposal = get_deal_proposal(&mut rt, deal_id);

    let v_response = verify_deals_for_activation(
        &mut rt,
        PROVIDER_ADDR,
        vec![SectorDeals {
            sector_type: RegisteredSealProof::StackedDRG2KiBV1P1,
            sector_expiry: SECTOR_EXPIRY,
            deal_ids: vec![deal_id],
        }],
        |_| None,
    );
    let a_response = activate_deals(&mut rt, SECTOR_EXPIRY, PROVIDER_ADDR, CURR_EPOCH, &[deal_id]);
    assert_eq!(1, v_response.sectors.len());
    assert_eq!(Some(make_piece_cid("1".as_bytes())), v_response.sectors[0].commd);
    assert!(a_response.verified_infos.is_empty());
    assert_eq!(BigInt::from(deal_proposal.piece_size.0), a_response.nonverified_deal_space);

    check_state(&rt);
}

#[test]
fn verify_deal_and_activate_to_get_deal_space_for_verified_deal_proposal() {
    let mut rt = setup();
    let next_allocation_id = 1;
    let deal_id = generate_and_publish_verified_deal(
        &mut rt,
        CLIENT_ADDR,
        &MINER_ADDRESSES,
        START_EPOCH,
        END_EPOCH,
        next_allocation_id,
    );
    let deal_proposal = get_deal_proposal(&mut rt, deal_id);

    let response = verify_deals_for_activation(
        &mut rt,
        PROVIDER_ADDR,
        vec![SectorDeals {
            sector_type: RegisteredSealProof::StackedDRG2KiBV1P1,
            sector_expiry: SECTOR_EXPIRY,
            deal_ids: vec![deal_id],
        }],
        |_| None,
    );

    let a_response = activate_deals(&mut rt, SECTOR_EXPIRY, PROVIDER_ADDR, CURR_EPOCH, &[deal_id]);

    assert_eq!(1, response.sectors.len());
    assert_eq!(Some(make_piece_cid("1".as_bytes())), response.sectors[0].commd);
    assert_eq!(1, a_response.verified_infos.len());
    assert_eq!(deal_proposal.piece_size, a_response.verified_infos[0].size);
    assert_eq!(deal_proposal.client.id().unwrap(), a_response.verified_infos[0].client);
    assert_eq!(deal_proposal.piece_cid, a_response.verified_infos[0].data);
    assert_eq!(next_allocation_id, a_response.verified_infos[0].allocation_id);

    assert_eq!(BigInt::zero(), a_response.nonverified_deal_space);

    check_state(&rt);
}

#[test]
fn verification_and_weights_for_verified_and_unverified_deals() {
    let mut rt = setup();
    let mut create_deal = |end_epoch, verified| {
        let mut deal = generate_deal_and_add_funds(
            &mut rt,
            CLIENT_ADDR,
            &MINER_ADDRESSES,
            START_EPOCH,
            end_epoch,
        );
        deal.verified_deal = verified;
        deal
    };

    let verified_deal_1 = create_deal(END_EPOCH, true);
    let verified_deal_2 = create_deal(END_EPOCH + 1, true);
    let unverified_deal_1 = create_deal(END_EPOCH + 2, false);
    let unverified_deal_2 = create_deal(END_EPOCH + 3, false);
    let deals = [
        verified_deal_1.clone(),
        verified_deal_2.clone(),
        unverified_deal_1.clone(),
        unverified_deal_2.clone(),
    ];

    rt.set_caller(*ACCOUNT_ACTOR_CODE_ID, WORKER_ADDR);
    let deal_ids = publish_deals(&mut rt, &MINER_ADDRESSES, &deals.clone(), 1);

    let response = verify_deals_for_activation(
        &mut rt,
        PROVIDER_ADDR,
        vec![SectorDeals {
            sector_type: RegisteredSealProof::StackedDRG8MiBV1,
            sector_expiry: SECTOR_EXPIRY,
            deal_ids: deal_ids.clone(),
        }],
        |_| {
            Some(
                deals
                    .iter()
                    .map(|deal| PieceInfo { size: deal.piece_size, cid: deal.piece_cid })
                    .collect(),
            )
        },
    );

    let verified_space = BigInt::from(verified_deal_1.piece_size.0 + verified_deal_2.piece_size.0);
    let unverified_space =
        BigInt::from(unverified_deal_1.piece_size.0 + unverified_deal_2.piece_size.0);

    let a_response = activate_deals(&mut rt, SECTOR_EXPIRY, PROVIDER_ADDR, CURR_EPOCH, &deal_ids);

    assert_eq!(1, response.sectors.len());
    let returned_verified_space: BigInt =
        a_response.verified_infos.iter().map(|info| BigInt::from(info.size.0)).sum();
    assert_eq!(verified_space, returned_verified_space);
    assert_eq!(unverified_space, a_response.nonverified_deal_space);

    check_state(&rt);
}

#[test]
fn fail_when_caller_is_not_a_storage_miner_actor() {
    let mut rt = setup();
    let deal_id =
        generate_and_publish_deal(&mut rt, CLIENT_ADDR, &MINER_ADDRESSES, START_EPOCH, END_EPOCH);

    rt.set_caller(*ACCOUNT_ACTOR_CODE_ID, WORKER_ADDR);
    rt.expect_validate_caller_type(vec![Type::Miner]);

    let params = VerifyDealsForActivationParams {
        sectors: vec![SectorDeals {
            sector_type: RegisteredSealProof::StackedDRG2KiBV1P1,
            sector_expiry: SECTOR_EXPIRY,
            deal_ids: vec![deal_id],
        }],
    };
    expect_abort(
        ExitCode::USR_FORBIDDEN,
        rt.call::<MarketActor>(
            Method::VerifyDealsForActivation as u64,
            IpldBlock::serialize_cbor(&params).unwrap(),
        ),
    );

    rt.verify();
    check_state(&rt);
}

#[test]
fn fail_when_deal_proposal_is_not_found() {
    let mut rt = setup();

    let params = VerifyDealsForActivationParams {
        sectors: vec![SectorDeals {
            sector_type: RegisteredSealProof::StackedDRG2KiBV1P1,
            sector_expiry: SECTOR_EXPIRY,
            deal_ids: vec![1],
        }],
    };
    rt.set_caller(*MINER_ACTOR_CODE_ID, PROVIDER_ADDR);
    rt.expect_validate_caller_type(vec![Type::Miner]);
    expect_abort(
        ExitCode::USR_NOT_FOUND,
        rt.call::<MarketActor>(
            Method::VerifyDealsForActivation as u64,
            IpldBlock::serialize_cbor(&params).unwrap(),
        ),
    );

    rt.verify();
    check_state(&rt);
}

#[test]
fn fail_when_caller_is_not_the_provider() {
    let mut rt = setup();
    let deal_id =
        generate_and_publish_deal(&mut rt, CLIENT_ADDR, &MINER_ADDRESSES, START_EPOCH, END_EPOCH);

    rt.set_caller(*MINER_ACTOR_CODE_ID, Address::new_id(205));
    rt.expect_validate_caller_type(vec![Type::Miner]);

    let params = VerifyDealsForActivationParams {
        sectors: vec![SectorDeals {
            sector_type: RegisteredSealProof::StackedDRG2KiBV1P1,
            sector_expiry: SECTOR_EXPIRY,
            deal_ids: vec![deal_id],
        }],
    };
    expect_abort(
        ExitCode::USR_FORBIDDEN,
        rt.call::<MarketActor>(
            Method::VerifyDealsForActivation as u64,
            IpldBlock::serialize_cbor(&params).unwrap(),
        ),
    );

    rt.verify();
    check_state(&rt);
}

#[test]
fn fail_when_current_epoch_is_greater_than_proposal_start_epoch() {
    let mut rt = setup();
    let deal_id =
        generate_and_publish_deal(&mut rt, CLIENT_ADDR, &MINER_ADDRESSES, START_EPOCH, END_EPOCH);
    rt.set_epoch(START_EPOCH + 1);

    rt.set_caller(*MINER_ACTOR_CODE_ID, PROVIDER_ADDR);
    rt.expect_validate_caller_type(vec![Type::Miner]);

    let params = VerifyDealsForActivationParams {
        sectors: vec![SectorDeals {
            sector_type: RegisteredSealProof::StackedDRG2KiBV1P1,
            sector_expiry: SECTOR_EXPIRY,
            deal_ids: vec![deal_id],
        }],
    };
    expect_abort(
        ExitCode::USR_ILLEGAL_ARGUMENT,
        rt.call::<MarketActor>(
            Method::VerifyDealsForActivation as u64,
            IpldBlock::serialize_cbor(&params).unwrap(),
        ),
    );

    rt.verify();
    check_state(&rt);
}

#[test]
fn fail_when_deal_end_epoch_is_greater_than_sector_expiration() {
    let mut rt = setup();
    let deal_id =
        generate_and_publish_deal(&mut rt, CLIENT_ADDR, &MINER_ADDRESSES, START_EPOCH, END_EPOCH);

    rt.set_caller(*MINER_ACTOR_CODE_ID, PROVIDER_ADDR);
    rt.expect_validate_caller_type(vec![Type::Miner]);

    let params = VerifyDealsForActivationParams {
        sectors: vec![SectorDeals {
            sector_type: RegisteredSealProof::StackedDRG2KiBV1P1,
            sector_expiry: END_EPOCH - 1,
            deal_ids: vec![deal_id],
        }],
    };
    expect_abort(
        ExitCode::USR_ILLEGAL_ARGUMENT,
        rt.call::<MarketActor>(
            Method::VerifyDealsForActivation as u64,
            IpldBlock::serialize_cbor(&params).unwrap(),
        ),
    );

    rt.verify();
    check_state(&rt);
}

#[test]
fn fail_when_the_same_deal_id_is_passed_multiple_times() {
    let mut rt = setup();
    let deal_id =
        generate_and_publish_deal(&mut rt, CLIENT_ADDR, &MINER_ADDRESSES, START_EPOCH, END_EPOCH);

    rt.set_caller(*MINER_ACTOR_CODE_ID, PROVIDER_ADDR);
    rt.expect_validate_caller_type(vec![Type::Miner]);

    let params = VerifyDealsForActivationParams {
        sectors: vec![SectorDeals {
            sector_type: RegisteredSealProof::StackedDRG8MiBV1,
            sector_expiry: SECTOR_EXPIRY,
            deal_ids: vec![deal_id, deal_id],
        }],
    };
    expect_abort_contains_message(
        ExitCode::USR_ILLEGAL_ARGUMENT,
        "multiple times",
        rt.call::<MarketActor>(
            Method::VerifyDealsForActivation as u64,
            IpldBlock::serialize_cbor(&params).unwrap(),
        ),
    );

    rt.verify();
    check_state(&rt);
}
