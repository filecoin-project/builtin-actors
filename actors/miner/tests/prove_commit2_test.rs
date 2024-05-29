use fvm_ipld_encoding::RawBytes;
use fvm_shared::error::ExitCode;
use fvm_shared::sector::SectorNumber;
use fvm_shared::{bigint::Zero, clock::ChainEpoch, econ::TokenAmount, ActorID};

use fil_actor_miner::ext::verifreg::{AllocationClaim, SectorAllocationClaims};
use fil_actor_miner::{
    DataActivationNotification, PieceChange, ProveCommitSectors3Return, SectorChanges,
    SectorOnChainInfo, SectorPreCommitInfo,
};
use fil_actors_runtime::cbor::serialize;
use fil_actors_runtime::test_utils::MockRuntime;
use fil_actors_runtime::{BatchReturn, EPOCHS_IN_DAY, STORAGE_MARKET_ACTOR_ADDR};
use util::*;

mod util;

const CLIENT_ID: ActorID = 1000;
const DEFAULT_SECTOR_EXPIRATION_DAYS: ChainEpoch = 220;
const FIRST_SECTOR_NUMBER: SectorNumber = 100;

#[test]
fn commit_batch() {
    let (h, mut rt) = setup_basic();
    let piece_size = h.sector_size as u64;
    let precommits = precommit_sectors(
        &mut rt,
        &h,
        &[&[piece_size], &[piece_size], &[piece_size], &[piece_size]],
    );
    let snos: Vec<SectorNumber> =
        precommits.iter().map(|pci: &SectorPreCommitInfo| pci.sector_number).collect();

    // Prove them in batch, each with a single piece.

    let manifests = vec![
        make_activation_manifest(snos[0], &[(piece_size, 0, 0, 0)]), // No alloc or deal
        make_activation_manifest(snos[1], &[(piece_size, CLIENT_ID, 1000, 0)]), // Just an alloc
        make_activation_manifest(snos[2], &[(piece_size, 0, 0, 2000)]), // Just a deal
        make_activation_manifest(snos[3], &[(piece_size, CLIENT_ID, 1001, 2001)]), // Alloc and deal
    ];

    let cfg = ProveCommitSectors3Config::default();
    let (result, claims, notifications) =
        h.prove_commit_sectors3(&rt, &manifests, true, true, false, cfg).unwrap();
    assert_commit_result(&[ExitCode::OK; 4], &result);
    let sectors: Vec<SectorOnChainInfo> = snos.iter().map(|sno| h.get_sector(&rt, *sno)).collect();

    // Explicitly verify claims match what we expect.
    assert_eq!(
        vec![
            SectorAllocationClaims {
                sector: snos[0],
                expiry: sectors[0].expiration,
                claims: vec![],
            },
            SectorAllocationClaims {
                sector: snos[1],
                expiry: sectors[1].expiration,
                claims: vec![AllocationClaim {
                    client: CLIENT_ID,
                    allocation_id: 1000,
                    data: manifests[1].pieces[0].cid,
                    size: manifests[1].pieces[0].size,
                }],
            },
            SectorAllocationClaims {
                sector: snos[2],
                expiry: sectors[2].expiration,
                claims: vec![],
            },
            SectorAllocationClaims {
                sector: snos[3],
                expiry: sectors[3].expiration,
                claims: vec![AllocationClaim {
                    client: CLIENT_ID,
                    allocation_id: 1001,
                    data: manifests[3].pieces[0].cid,
                    size: manifests[3].pieces[0].size,
                }],
            },
        ],
        claims
    );

    // Explicitly verify notifications match what we expect.
    assert_eq!(
        vec![
            SectorChanges {
                sector: snos[2],
                minimum_commitment_epoch: sectors[2].expiration,
                added: vec![PieceChange {
                    data: manifests[2].pieces[0].cid,
                    size: manifests[2].pieces[0].size,
                    payload: serialize(&2000, "").unwrap(),
                },],
            },
            SectorChanges {
                sector: snos[3],
                minimum_commitment_epoch: sectors[3].expiration,
                added: vec![PieceChange {
                    data: manifests[3].pieces[0].cid,
                    size: manifests[3].pieces[0].size,
                    payload: serialize(&2001, "").unwrap(),
                },],
            },
        ],
        notifications
    );

    // Sector 0: Even though there's no "deal", the data weight is set.
    verify_weights(&rt, &h, snos[0], piece_size, 0);
    // Sector 1: With an allocation, the verified weight is set instead.
    verify_weights(&rt, &h, snos[1], 0, piece_size);
    // Sector 2: Deal weight is set.
    verify_weights(&rt, &h, snos[2], piece_size, 0);
    // Sector 3: Deal doesn't make a difference to verified weight only set.
    verify_weights(&rt, &h, snos[3], 0, piece_size);
    h.check_state(&rt);
}

#[test]
fn multiple_pieces_in_sector() {
    let (h, mut rt) = setup_basic();
    // Half-size pieces
    let piece_size = h.sector_size as u64 / 2;
    let precommits =
        precommit_sectors(&mut rt, &h, &[&[piece_size, piece_size], &[piece_size, piece_size]]);
    let snos: Vec<SectorNumber> =
        precommits.iter().map(|pci: &SectorPreCommitInfo| pci.sector_number).collect();

    let manifests = vec![
        make_activation_manifest(
            snos[0],
            &[(piece_size, CLIENT_ID, 1000, 2000), (piece_size, CLIENT_ID, 1001, 2001)],
        ),
        make_activation_manifest(
            snos[1],
            &[(piece_size, CLIENT_ID, 1002, 2002), (piece_size, 0, 0, 0)],
        ),
    ];

    let cfg = ProveCommitSectors3Config::default();
    let (result, claims, notifications) =
        h.prove_commit_sectors3(&rt, &manifests, true, true, false, cfg).unwrap();
    assert_commit_result(&[ExitCode::OK, ExitCode::OK], &result);
    let sectors: Vec<SectorOnChainInfo> = snos.iter().map(|sno| h.get_sector(&rt, *sno)).collect();

    // Explicitly verify claims match what we expect.
    assert_eq!(
        vec![
            SectorAllocationClaims {
                sector: snos[0],
                expiry: sectors[0].expiration,
                claims: vec![
                    AllocationClaim {
                        client: CLIENT_ID,
                        allocation_id: 1000,
                        data: manifests[0].pieces[0].cid,
                        size: manifests[0].pieces[0].size,
                    },
                    AllocationClaim {
                        client: CLIENT_ID,
                        allocation_id: 1001,
                        data: manifests[0].pieces[1].cid,
                        size: manifests[0].pieces[1].size,
                    },
                ],
            },
            SectorAllocationClaims {
                sector: snos[1],
                expiry: sectors[1].expiration,
                claims: vec![AllocationClaim {
                    client: CLIENT_ID,
                    allocation_id: 1002,
                    data: manifests[1].pieces[0].cid,
                    size: manifests[1].pieces[0].size,
                }],
            },
        ],
        claims
    );

    // Explicitly verify notifications match what we expect.
    assert_eq!(
        vec![
            SectorChanges {
                sector: snos[0],
                minimum_commitment_epoch: sectors[0].expiration,
                added: vec![
                    PieceChange {
                        data: manifests[0].pieces[0].cid,
                        size: manifests[0].pieces[0].size,
                        payload: serialize(&2000, "").unwrap(),
                    },
                    PieceChange {
                        data: manifests[0].pieces[1].cid,
                        size: manifests[0].pieces[1].size,
                        payload: serialize(&2001, "").unwrap(),
                    },
                ],
            },
            SectorChanges {
                sector: snos[1],
                minimum_commitment_epoch: sectors[1].expiration,
                added: vec![PieceChange {
                    data: manifests[1].pieces[0].cid,
                    size: manifests[1].pieces[0].size,
                    payload: serialize(&2002, "").unwrap(),
                },],
            },
        ],
        notifications
    );

    verify_weights(&rt, &h, snos[0], 0, piece_size * 2);
    verify_weights(&rt, &h, snos[1], piece_size, piece_size);
    h.check_state(&rt);
}

#[test]
fn multiple_notifs_for_piece() {
    let (h, mut rt) = setup_basic();
    let piece_size = h.sector_size as u64 / 2;
    let precommits = precommit_sectors(&mut rt, &h, &[&[piece_size, piece_size], &[piece_size]]);
    let snos: Vec<SectorNumber> =
        precommits.iter().map(|pci: &SectorPreCommitInfo| pci.sector_number).collect();

    let mut manifests = vec![
        make_activation_manifest(
            snos[0],
            &[(piece_size, CLIENT_ID, 0, 0), (piece_size, CLIENT_ID, 0, 0)],
        ),
        make_activation_manifest(snos[1], &[(piece_size, CLIENT_ID, 0, 0)]),
    ];
    // 2 notifications for sector[0], piece[0]
    manifests[0].pieces[0].notify.push(DataActivationNotification {
        address: STORAGE_MARKET_ACTOR_ADDR,
        payload: RawBytes::from(vec![6, 6, 6, 6]),
    });
    manifests[0].pieces[0].notify.push(DataActivationNotification {
        address: STORAGE_MARKET_ACTOR_ADDR,
        payload: RawBytes::from(vec![7, 7, 7, 7]),
    });
    // One notification for sector[0], piece[1]
    manifests[0].pieces[1].notify.push(DataActivationNotification {
        address: STORAGE_MARKET_ACTOR_ADDR,
        payload: RawBytes::from(vec![8, 8, 8, 8]),
    });
    // One notification for sector[1], piece[0]
    manifests[1].pieces[0].notify.push(DataActivationNotification {
        address: STORAGE_MARKET_ACTOR_ADDR,
        payload: RawBytes::from(vec![9, 9, 9, 9]),
    });

    let cfg = ProveCommitSectors3Config::default();
    let (result, _, notifications) =
        h.prove_commit_sectors3(&rt, &manifests, true, true, false, cfg).unwrap();
    assert_commit_result(&[ExitCode::OK, ExitCode::OK], &result);
    let sectors: Vec<SectorOnChainInfo> = snos.iter().map(|sno| h.get_sector(&rt, *sno)).collect();

    // Explicitly verify notifications match what we expect.
    assert_eq!(
        vec![
            SectorChanges {
                sector: snos[0],
                minimum_commitment_epoch: sectors[0].expiration,
                added: vec![
                    PieceChange {
                        data: manifests[0].pieces[0].cid,
                        size: manifests[0].pieces[0].size,
                        payload: RawBytes::from(vec![6, 6, 6, 6]),
                    },
                    PieceChange {
                        data: manifests[0].pieces[0].cid,
                        size: manifests[0].pieces[0].size,
                        payload: RawBytes::from(vec![7, 7, 7, 7]),
                    },
                    PieceChange {
                        data: manifests[0].pieces[1].cid,
                        size: manifests[0].pieces[1].size,
                        payload: RawBytes::from(vec![8, 8, 8, 8]),
                    },
                ],
            },
            SectorChanges {
                sector: snos[1],
                minimum_commitment_epoch: sectors[1].expiration,
                added: vec![PieceChange {
                    data: manifests[1].pieces[0].cid,
                    size: manifests[1].pieces[0].size,
                    payload: RawBytes::from(vec![9, 9, 9, 9]),
                },],
            },
        ],
        notifications
    );

    verify_weights(&rt, &h, snos[0], piece_size * 2, 0);
    verify_weights(&rt, &h, snos[1], piece_size, 0);
    h.check_state(&rt);
}

#[test]
fn expired_precommit_dropped_batch() {
    let (h, mut rt) = setup_basic();
    let piece_size = h.sector_size as u64;
    let precommits1 = precommit_sectors(&mut rt, &h, &[&[piece_size]]);
    let epoch = *rt.epoch.borrow();
    rt.set_epoch(epoch + 31 * EPOCHS_IN_DAY); // The first precommit expired.

    let precommits2 = precommit_sectors_from(
        &mut rt,
        &h,
        precommits1[0].sector_number + 1,
        &[&[piece_size]],
        false,
    );
    let precommits = [&precommits1[..], &precommits2[..]].concat();
    let snos: Vec<SectorNumber> =
        precommits.iter().map(|pci: &SectorPreCommitInfo| pci.sector_number).collect();

    let manifests = vec![
        make_activation_manifest(snos[0], &[(piece_size, CLIENT_ID, 1000, 2000)]),
        make_activation_manifest(snos[1], &[(piece_size, CLIENT_ID, 1001, 2001)]),
    ];

    let cfg = ProveCommitSectors3Config { validation_failure: vec![0], ..Default::default() };
    let (result, claims, notifications) =
        h.prove_commit_sectors3(&rt, &manifests, false, false, false, cfg).unwrap();
    assert_commit_result(&[ExitCode::USR_ILLEGAL_ARGUMENT, ExitCode::OK], &result);

    // Sector 0: not committed
    assert_eq!(precommits1[0], h.get_precommit(&rt, snos[0]).info);
    let st = h.get_state(&rt);
    assert!(st.get_sector(&rt.store, precommits[0].sector_number).unwrap().is_none());
    // Sector 1: verified weight.
    verify_weights(&rt, &h, snos[1], 0, piece_size);
    assert_eq!(1, claims.len());
    assert_eq!(precommits[1].sector_number, claims[0].sector);
    assert_eq!(1, notifications.len());
    assert_eq!(precommits[1].sector_number, notifications[0].sector);
    h.check_state(&rt);
}

#[test]
fn expired_precommit_dropped_aggregate() {
    let (h, mut rt) = setup_basic();
    let piece_size = h.sector_size as u64;
    let precommits1 = precommit_sectors(&mut rt, &h, &[&[piece_size]]);
    let epoch = *rt.epoch.borrow();
    rt.set_epoch(epoch + 31 * EPOCHS_IN_DAY); // The first precommit expired.

    let precommits2 = precommit_sectors_from(
        &mut rt,
        &h,
        precommits1[0].sector_number + 1,
        &[&[piece_size], &[piece_size], &[piece_size]],
        false,
    );
    let precommits = [&precommits1[..], &precommits2[..]].concat();
    let snos: Vec<SectorNumber> =
        precommits.iter().map(|pci: &SectorPreCommitInfo| pci.sector_number).collect();

    let manifests = vec![
        make_activation_manifest(snos[0], &[(piece_size, CLIENT_ID, 1000, 2000)]),
        make_activation_manifest(snos[1], &[(piece_size, CLIENT_ID, 1001, 2001)]),
        make_activation_manifest(snos[2], &[(piece_size, CLIENT_ID, 1002, 2002)]),
        make_activation_manifest(snos[3], &[(piece_size, CLIENT_ID, 1003, 2003)]),
    ];

    let cfg = ProveCommitSectors3Config { validation_failure: vec![0], ..Default::default() };
    let (result, _, _) = h.prove_commit_sectors3(&rt, &manifests, false, false, true, cfg).unwrap();
    assert_commit_result(
        &[ExitCode::USR_ILLEGAL_ARGUMENT, ExitCode::OK, ExitCode::OK, ExitCode::OK],
        &result,
    );

    // Sector 0: not committed
    assert_eq!(precommits1[0], h.get_precommit(&rt, snos[0]).info);
    let st = h.get_state(&rt);
    assert!(st.get_sector(&rt.store, precommits[0].sector_number).unwrap().is_none());
    // Sectors 1-3: verified weight.
    verify_weights(&rt, &h, snos[1], 0, piece_size);
    verify_weights(&rt, &h, snos[2], 0, piece_size);
    verify_weights(&rt, &h, snos[3], 0, piece_size);
    h.check_state(&rt);
}

#[test]
fn invalid_proof_dropped() {
    let (h, mut rt) = setup_basic();
    let piece_size = h.sector_size as u64;
    let precommits = precommit_sectors(&mut rt, &h, &[&[piece_size], &[piece_size]]);
    let snos: Vec<SectorNumber> =
        precommits.iter().map(|pci: &SectorPreCommitInfo| pci.sector_number).collect();

    let manifests = vec![
        make_activation_manifest(snos[0], &[(piece_size, CLIENT_ID, 1000, 2000)]),
        make_activation_manifest(snos[1], &[(piece_size, CLIENT_ID, 1001, 2001)]),
    ];

    let cfg = ProveCommitSectors3Config { proof_failure: vec![0], ..Default::default() };
    let (result, _, _) =
        h.prove_commit_sectors3(&rt, &manifests, false, false, false, cfg).unwrap();
    assert_commit_result(&[ExitCode::USR_ILLEGAL_ARGUMENT, ExitCode::OK], &result);

    // Sector 0: not committed
    let st = h.get_state(&rt);
    assert!(st.get_sector(&rt.store, precommits[0].sector_number).unwrap().is_none());
    // Sector 1: verified weight.
    verify_weights(&rt, &h, snos[1], 0, piece_size);
    h.check_state(&rt);
}

#[test]
fn invalid_claim_dropped() {
    let (h, mut rt) = setup_basic();
    let piece_size = h.sector_size as u64;
    let precommits = precommit_sectors(&mut rt, &h, &[&[piece_size], &[piece_size]]);
    let snos: Vec<SectorNumber> =
        precommits.iter().map(|pci: &SectorPreCommitInfo| pci.sector_number).collect();

    let manifests = vec![
        make_activation_manifest(snos[0], &[(piece_size, CLIENT_ID, 1000, 2000)]),
        make_activation_manifest(snos[1], &[(piece_size, CLIENT_ID, 1001, 2001)]),
    ];

    let cfg = ProveCommitSectors3Config { claim_failure: vec![0], ..Default::default() };
    let (result, _, _) =
        h.prove_commit_sectors3(&rt, &manifests, false, false, false, cfg).unwrap();
    assert_commit_result(&[ExitCode::USR_ILLEGAL_ARGUMENT, ExitCode::OK], &result);

    // Sector 0: not committed
    let st = h.get_state(&rt);
    assert!(st.get_sector(&rt.store, precommits[0].sector_number).unwrap().is_none());
    // Sector 1: verified weight.
    verify_weights(&rt, &h, snos[1], 0, piece_size);
}

#[test]
fn aborted_notification_dropped() {
    let (h, mut rt) = setup_basic();
    let piece_size = h.sector_size as u64;
    let precommits = precommit_sectors(&mut rt, &h, &[&[piece_size], &[piece_size]]);
    let snos: Vec<SectorNumber> =
        precommits.iter().map(|pci: &SectorPreCommitInfo| pci.sector_number).collect();

    let manifests = vec![
        make_activation_manifest(snos[0], &[(piece_size, CLIENT_ID, 1000, 2000)]),
        make_activation_manifest(snos[1], &[(piece_size, CLIENT_ID, 1001, 2001)]),
    ];

    let cfg = ProveCommitSectors3Config {
        notification_result: Some(ExitCode::USR_UNSPECIFIED),
        ..Default::default()
    };
    let (result, _, _) =
        h.prove_commit_sectors3(&rt, &manifests, false, false, false, cfg).unwrap();

    // All sectors succeed anyway.
    assert_commit_result(&[ExitCode::OK; 2], &result);
    // All power activated anyway.
    verify_weights(&rt, &h, snos[0], 0, piece_size);
    verify_weights(&rt, &h, snos[1], 0, piece_size);
}

#[test]
fn rejected_notification_dropped() {
    let (h, mut rt) = setup_basic();
    let piece_size = h.sector_size as u64;
    let precommits = precommit_sectors(&mut rt, &h, &[&[piece_size], &[piece_size]]);
    let snos: Vec<SectorNumber> =
        precommits.iter().map(|pci: &SectorPreCommitInfo| pci.sector_number).collect();

    let manifests = vec![
        make_activation_manifest(snos[0], &[(piece_size, CLIENT_ID, 1000, 2000)]),
        make_activation_manifest(snos[1], &[(piece_size, CLIENT_ID, 1001, 2001)]),
    ];

    let cfg = ProveCommitSectors3Config { notification_rejected: true, ..Default::default() };
    let (result, _, _) =
        h.prove_commit_sectors3(&rt, &manifests, false, false, false, cfg).unwrap();

    // All sectors succeed anyway.
    assert_commit_result(&[ExitCode::OK; 2], &result);
    // All power activated anyway.
    verify_weights(&rt, &h, snos[0], 0, piece_size);
    verify_weights(&rt, &h, snos[1], 0, piece_size);
}

fn setup_basic() -> (ActorHarness, MockRuntime) {
    let h = ActorHarness::new_with_options(HarnessOptions::default());
    let rt = h.new_runtime();
    rt.set_balance(BIG_BALANCE.clone());
    h.construct_and_verify(&rt);
    (h, rt)
}

fn precommit_sectors(
    rt: &mut MockRuntime,
    h: &ActorHarness,
    piece_sizes: &[&[u64]],
) -> Vec<SectorPreCommitInfo> {
    precommit_sectors_from(rt, h, FIRST_SECTOR_NUMBER, piece_sizes, true)
}

fn precommit_sectors_from(
    rt: &mut MockRuntime,
    h: &ActorHarness,
    first_sector_number: SectorNumber,
    piece_sizes: &[&[u64]],
    first_for_miner: bool,
) -> Vec<SectorPreCommitInfo> {
    let precommit_epoch = *rt.epoch.borrow();
    let sector_expiry = *rt.epoch.borrow() + DEFAULT_SECTOR_EXPIRATION_DAYS * EPOCHS_IN_DAY;
    let precommits = make_fake_precommits(
        h,
        first_sector_number,
        precommit_epoch - 1,
        sector_expiry,
        piece_sizes,
    );
    h.pre_commit_sector_batch_v2(rt, &precommits, first_for_miner, &TokenAmount::zero()).unwrap();
    rt.set_epoch(precommit_epoch + rt.policy.pre_commit_challenge_delay + 1);
    precommits
}

fn assert_commit_result(expected: &[ExitCode], result: &ProveCommitSectors3Return) {
    assert_eq!(BatchReturn::of(expected), result.activation_results);
}
