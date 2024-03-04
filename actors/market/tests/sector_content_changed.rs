use cid::Cid;
use fvm_ipld_encoding::ipld_block::IpldBlock;
use fvm_shared::clock::ChainEpoch;
use fvm_shared::deal::DealID;
use fvm_shared::econ::TokenAmount;
use fvm_shared::error::ExitCode;
use fvm_shared::piece::PaddedPieceSize;
use multihash::Code::Sha2_256;
use multihash::MultihashDigest;
use num_traits::Zero;

use fil_actor_market::ext::miner::{
    PieceChange, PieceReturn, SectorChanges, SectorContentChangedParams,
};
use fil_actor_market::{DealProposal, Method, NO_ALLOCATION_ID};
use fil_actors_runtime::cbor::serialize;
use fil_actors_runtime::runtime::builtins::Type;
use fil_actors_runtime::test_utils::{expect_abort, MockRuntime, ACCOUNT_ACTOR_CODE_ID};
use fil_actors_runtime::EPOCHS_IN_DAY;
use harness::*;

mod harness;

const START_EPOCH: ChainEpoch = 10;
const END_EPOCH: ChainEpoch = 200 * EPOCHS_IN_DAY;
const MINER_ADDRESSES: MinerAddresses = MinerAddresses {
    owner: OWNER_ADDR,
    worker: WORKER_ADDR,
    provider: PROVIDER_ADDR,
    control: vec![],
};

// These tests share a lot in common with those for BatchActivateDeals,
// as they perform similar functions.

#[test]
fn empty_params() {
    let rt = setup();

    // Empty params
    let changes = vec![];
    let ret = sector_content_changed(&rt, PROVIDER_ADDR, changes).unwrap();
    assert_eq!(0, ret.sectors.len());

    // Sector with no pieces
    let changes =
        vec![SectorChanges { sector: 1, minimum_commitment_epoch: END_EPOCH, added: vec![] }];
    let ret = sector_content_changed(&rt, PROVIDER_ADDR, changes).unwrap();
    assert_eq!(1, ret.sectors.len());
    assert_eq!(0, ret.sectors[0].added.len());
    check_state(&rt);
}

#[test]
fn simple_one_sector() {
    let rt = setup();
    let epoch = rt.set_epoch(START_EPOCH);
    let mut deals = create_deals(&rt, 3);
    deals[2].verified_deal = true;

    let next_allocation_id = 1;
    let datacap_required = TokenAmount::from_whole(deals[2].piece_size.0);
    rt.set_caller(*ACCOUNT_ACTOR_CODE_ID, WORKER_ADDR);
    let deal_ids =
        publish_deals(&rt, &MINER_ADDRESSES, &deals, datacap_required, next_allocation_id);

    let mut pieces = pieces_from_deals(&deal_ids, &deals);
    pieces.reverse();

    let sno = 7;
    let changes = vec![SectorChanges {
        sector: sno,
        minimum_commitment_epoch: END_EPOCH + 10,
        added: pieces,
    }];
    for deal_id in deal_ids.iter().rev() {
        harness::expect_emitted(
            &rt,
            "deal-activated",
            *deal_id,
            CLIENT_ADDR.id().unwrap(),
            MINER_ADDRESSES.provider.id().unwrap(),
        );
    }
    let ret = sector_content_changed(&rt, PROVIDER_ADDR, changes).unwrap();
    assert_eq!(1, ret.sectors.len());
    assert_eq!(3, ret.sectors[0].added.len());
    assert!(ret.sectors[0].added.iter().all(|r| r.accepted));

    // Deal IDs are stored under the sector, in correct order.
    assert_eq!(deal_ids, get_sector_deal_ids(&rt, PROVIDER_ID, &[sno]));

    // Deal states include allocation IDs from when they were published.
    for id in deal_ids.iter() {
        let state = get_deal_state(&rt, *id);
        assert_eq!(sno, state.sector_number);
        assert_eq!(epoch, state.sector_start_epoch);
    }
    check_state(&rt);
}

#[test]
fn simple_multiple_sectors() {
    let rt = setup();
    let deals = create_deals(&rt, 3);
    rt.set_caller(*ACCOUNT_ACTOR_CODE_ID, WORKER_ADDR);
    let deal_ids =
        publish_deals(&rt, &MINER_ADDRESSES, &deals, TokenAmount::zero(), NO_ALLOCATION_ID);
    let pieces = pieces_from_deals(&deal_ids, &deals);

    let changes = vec![
        SectorChanges {
            sector: 1,
            minimum_commitment_epoch: END_EPOCH + 10,
            added: pieces[0..1].to_vec(),
        },
        // Same sector referenced twice, it's ok.
        SectorChanges {
            sector: 1,
            minimum_commitment_epoch: END_EPOCH + 10,
            added: pieces[1..2].to_vec(),
        },
        SectorChanges {
            sector: 2,
            minimum_commitment_epoch: END_EPOCH + 10,
            added: pieces[2..3].to_vec(),
        },
    ];
    for deal_id in deal_ids.iter() {
        harness::expect_emitted(
            &rt,
            "deal-activated",
            *deal_id,
            CLIENT_ADDR.id().unwrap(),
            MINER_ADDRESSES.provider.id().unwrap(),
        );
    }
    let ret = sector_content_changed(&rt, PROVIDER_ADDR, changes).unwrap();
    assert_eq!(3, ret.sectors.len());
    assert_eq!(vec![PieceReturn { accepted: true }], ret.sectors[0].added);
    assert_eq!(vec![PieceReturn { accepted: true }], ret.sectors[1].added);
    assert_eq!(vec![PieceReturn { accepted: true }], ret.sectors[2].added);

    // Deal IDs are stored under the right sector, in correct order.
    assert_eq!(deal_ids[0..2], get_sector_deal_ids(&rt, PROVIDER_ID, &[1]));
    assert_eq!(deal_ids[2..3], get_sector_deal_ids(&rt, PROVIDER_ID, &[2]));
}

#[test]
fn new_deal_existing_sector() {
    let rt = setup();
    let deals = create_deals(&rt, 3);
    rt.set_caller(*ACCOUNT_ACTOR_CODE_ID, WORKER_ADDR);
    let deal_ids =
        publish_deals(&rt, &MINER_ADDRESSES, &deals, TokenAmount::zero(), NO_ALLOCATION_ID);
    let pieces = pieces_from_deals(&deal_ids, &deals);

    let changes = vec![SectorChanges {
        sector: 1,
        minimum_commitment_epoch: END_EPOCH + 10,
        added: pieces[1..3].to_vec(),
    }];
    for deal_id in deal_ids[1..3].iter() {
        harness::expect_emitted(
            &rt,
            "deal-activated",
            *deal_id,
            CLIENT_ADDR.id().unwrap(),
            MINER_ADDRESSES.provider.id().unwrap(),
        );
    }
    sector_content_changed(&rt, PROVIDER_ADDR, changes).unwrap();

    let changes = vec![SectorChanges {
        sector: 1,
        minimum_commitment_epoch: END_EPOCH + 10,
        added: pieces[0..1].to_vec(),
    }];
    for deal_id in deal_ids[0..1].iter() {
        harness::expect_emitted(
            &rt,
            "deal-activated",
            *deal_id,
            CLIENT_ADDR.id().unwrap(),
            MINER_ADDRESSES.provider.id().unwrap(),
        );
    }
    sector_content_changed(&rt, PROVIDER_ADDR, changes).unwrap();

    // All deal IDs are stored under the right sector, in correct order.
    assert_eq!(deal_ids[0..3], get_sector_deal_ids(&rt, PROVIDER_ID, &[1]));
}

#[test]
fn piece_must_match_deal() {
    let rt = setup();
    let deals = create_deals(&rt, 2);

    rt.set_caller(*ACCOUNT_ACTOR_CODE_ID, WORKER_ADDR);
    let deal_ids =
        publish_deals(&rt, &MINER_ADDRESSES, &deals, TokenAmount::zero(), NO_ALLOCATION_ID);
    let mut pieces = pieces_from_deals(&deal_ids, &deals);
    // Wrong CID
    pieces[0].data = Cid::new_v1(0, Sha2_256.digest(&[1, 2, 3, 4]));
    // Wrong size
    pieces[1].size = PaddedPieceSize(1234);
    // Deal doesn't exist
    pieces.push(PieceChange {
        data: Cid::new_v1(0, Sha2_256.digest(&[1, 2, 3, 4])),
        size: PaddedPieceSize(1234),
        payload: serialize(&1234, "deal id").unwrap(),
    });

    let changes =
        vec![SectorChanges { sector: 1, minimum_commitment_epoch: END_EPOCH + 10, added: pieces }];
    let ret = sector_content_changed(&rt, PROVIDER_ADDR, changes).unwrap();
    assert_eq!(1, ret.sectors.len());
    assert_eq!(
        vec![
            PieceReturn { accepted: false },
            PieceReturn { accepted: false },
            PieceReturn { accepted: false },
        ],
        ret.sectors[0].added
    );

    check_state(&rt);
}

#[test]
fn invalid_deal_id_rejected() {
    let rt = setup();
    let deals = create_deals(&rt, 1);

    rt.set_caller(*ACCOUNT_ACTOR_CODE_ID, WORKER_ADDR);
    let deal_ids =
        publish_deals(&rt, &MINER_ADDRESSES, &deals, TokenAmount::zero(), NO_ALLOCATION_ID);
    let mut pieces = pieces_from_deals(&deal_ids, &deals);
    // Append a byte to the deal ID.
    let mut buf = pieces[0].payload.to_vec();
    buf.push(123);
    pieces[0].payload = buf.into();

    let changes =
        vec![SectorChanges { sector: 1, minimum_commitment_epoch: END_EPOCH + 10, added: pieces }];
    let ret = sector_content_changed(&rt, PROVIDER_ADDR, changes).unwrap();
    assert_eq!(1, ret.sectors.len());
    assert_eq!(vec![PieceReturn { accepted: false },], ret.sectors[0].added);

    check_state(&rt);
}

#[test]
fn failures_isolated() {
    let rt = setup();
    let deals = create_deals(&rt, 4);
    rt.set_caller(*ACCOUNT_ACTOR_CODE_ID, WORKER_ADDR);
    let deal_ids =
        publish_deals(&rt, &MINER_ADDRESSES, &deals, TokenAmount::zero(), NO_ALLOCATION_ID);
    let mut pieces = pieces_from_deals(&deal_ids, &deals);

    // Break second and third pieces.
    pieces[1].size = PaddedPieceSize(1234);
    pieces[2].size = PaddedPieceSize(1234);
    let changes = vec![
        SectorChanges {
            sector: 1,
            minimum_commitment_epoch: END_EPOCH + 10,
            added: pieces[0..2].to_vec(),
        },
        SectorChanges {
            sector: 2,
            minimum_commitment_epoch: END_EPOCH + 10,
            added: pieces[2..3].to_vec(),
        },
        SectorChanges {
            sector: 3,
            minimum_commitment_epoch: END_EPOCH + 10,
            added: pieces[3..4].to_vec(),
        },
    ];

    // only first and last pieces emit an event
    for deal_id in [deal_ids.first().unwrap(), deal_ids.last().unwrap()] {
        harness::expect_emitted(
            &rt,
            "deal-activated",
            *deal_id,
            CLIENT_ADDR.id().unwrap(),
            MINER_ADDRESSES.provider.id().unwrap(),
        );
    }
    let ret = sector_content_changed(&rt, PROVIDER_ADDR, changes).unwrap();
    assert_eq!(3, ret.sectors.len());
    // Broken second piece still allows first piece in same sector to activate.
    assert_eq!(
        vec![PieceReturn { accepted: true }, PieceReturn { accepted: false }],
        ret.sectors[0].added
    );
    // Broken third piece
    assert_eq!(vec![PieceReturn { accepted: false }], ret.sectors[1].added);
    // Ok fourth piece.
    assert_eq!(vec![PieceReturn { accepted: true }], ret.sectors[2].added);

    // Successful deal IDs are stored under the right sector, in correct order.
    assert_eq!(deal_ids[0..1], get_sector_deal_ids(&rt, PROVIDER_ID, &[1]));
    assert_eq!(deal_ids[3..4], get_sector_deal_ids(&rt, PROVIDER_ID, &[3]));
}

#[test]
fn rejects_duplicates_in_same_sector() {
    let rt = setup();
    let deals = create_deals(&rt, 2);
    rt.set_caller(*ACCOUNT_ACTOR_CODE_ID, WORKER_ADDR);
    let deal_ids =
        publish_deals(&rt, &MINER_ADDRESSES, &deals, TokenAmount::zero(), NO_ALLOCATION_ID);
    let pieces = pieces_from_deals(&deal_ids, &deals);

    let changes = vec![
        // Same deal twice in one sector change.
        SectorChanges {
            sector: 1,
            minimum_commitment_epoch: END_EPOCH + 10,
            added: vec![pieces[0].clone(), pieces[0].clone(), pieces[1].clone()],
        },
    ];
    for deal_id in deal_ids.iter() {
        harness::expect_emitted(
            &rt,
            "deal-activated",
            *deal_id,
            CLIENT_ADDR.id().unwrap(),
            MINER_ADDRESSES.provider.id().unwrap(),
        );
    }
    let ret = sector_content_changed(&rt, PROVIDER_ADDR, changes).unwrap();
    assert_eq!(1, ret.sectors.len());
    // The first piece succeeds just once, the second piece succeeds too.
    assert_eq!(
        vec![
            PieceReturn { accepted: true },
            PieceReturn { accepted: false },
            PieceReturn { accepted: true },
        ],
        ret.sectors[0].added
    );

    // Deal IDs are stored under the right sector, in correct order.
    assert_eq!(deal_ids[0..2], get_sector_deal_ids(&rt, PROVIDER_ID, &[1]));
    assert_eq!(Vec::<DealID>::new(), get_sector_deal_ids(&rt, PROVIDER_ID, &[2]));
}

#[test]
fn rejects_duplicates_across_sectors() {
    let rt = setup();
    let deals = create_deals(&rt, 3);
    rt.set_caller(*ACCOUNT_ACTOR_CODE_ID, WORKER_ADDR);
    let deal_ids =
        publish_deals(&rt, &MINER_ADDRESSES, &deals, TokenAmount::zero(), NO_ALLOCATION_ID);
    let pieces = pieces_from_deals(&deal_ids, &deals);

    let changes = vec![
        SectorChanges {
            sector: 1,
            minimum_commitment_epoch: END_EPOCH + 10,
            added: vec![pieces[0].clone()],
        },
        // Same piece again, referencing same sector, plus a new piece.
        SectorChanges {
            sector: 1,
            minimum_commitment_epoch: END_EPOCH + 10,
            added: vec![pieces[0].clone(), pieces[1].clone()],
        },
        // Same deal piece in a different sector, plus second piece agoin, plus a new piece.
        SectorChanges {
            sector: 2,
            minimum_commitment_epoch: END_EPOCH + 10,
            added: vec![pieces[0].clone(), pieces[1].clone(), pieces[2].clone()],
        },
    ];
    for deal_id in deal_ids.iter() {
        harness::expect_emitted(
            &rt,
            "deal-activated",
            *deal_id,
            CLIENT_ADDR.id().unwrap(),
            MINER_ADDRESSES.provider.id().unwrap(),
        );
    }
    let ret = sector_content_changed(&rt, PROVIDER_ADDR, changes).unwrap();
    assert_eq!(3, ret.sectors.len());
    // Succeeds in the first time.
    assert_eq!(vec![PieceReturn { accepted: true },], ret.sectors[0].added);
    // Fails second time, but other piece succeeds.
    assert_eq!(
        vec![PieceReturn { accepted: false }, PieceReturn { accepted: true },],
        ret.sectors[1].added
    );
    // Both duplicates fail, but third piece succeeds.
    assert_eq!(
        vec![
            PieceReturn { accepted: false },
            PieceReturn { accepted: false },
            PieceReturn { accepted: true },
        ],
        ret.sectors[2].added
    );

    // Deal IDs are stored under the right sector, in correct order.
    assert_eq!(deal_ids[0..2], get_sector_deal_ids(&rt, PROVIDER_ID, &[1]));
    assert_eq!(deal_ids[2..3], get_sector_deal_ids(&rt, PROVIDER_ID, &[2]));
}

#[test]
fn require_miner_caller() {
    let rt = setup();
    let changes = vec![];
    rt.set_caller(*ACCOUNT_ACTOR_CODE_ID, PROVIDER_ADDR); // Not a miner
    rt.expect_validate_caller_type(vec![Type::Miner]);
    let params = SectorContentChangedParams { sectors: changes };

    expect_abort(
        ExitCode::USR_FORBIDDEN,
        rt.call::<fil_actor_market::Actor>(
            Method::SectorContentChangedExported as u64,
            IpldBlock::serialize_cbor(&params).unwrap(),
        ),
    );
}

fn create_deals(rt: &MockRuntime, count: i64) -> Vec<DealProposal> {
    (0..count)
        .map(|i| create_deal(rt, CLIENT_ADDR, &MINER_ADDRESSES, START_EPOCH, END_EPOCH + i, false))
        .collect()
}

fn pieces_from_deals(deal_ids: &[DealID], deals: &[DealProposal]) -> Vec<PieceChange> {
    deal_ids.iter().zip(deals).map(|(id, deal)| piece_info_from_deal(*id, deal)).collect()
}

// TODO

// - See activate_deals_failures
// - test bad deal ID, serialise
