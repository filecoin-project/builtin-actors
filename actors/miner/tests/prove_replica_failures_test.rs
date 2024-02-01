use fvm_ipld_encoding::RawBytes;
use fvm_shared::address::Address;
use fvm_shared::clock::ChainEpoch;
use fvm_shared::deal::DealID;
use fvm_shared::error::ExitCode;
use fvm_shared::sector::RegisteredAggregateProof::SnarkPackV2;
use fvm_shared::sector::SectorNumber;
use fvm_shared::ActorID;

use fil_actor_miner::ext::verifreg::AllocationID;
use fil_actor_miner::{
    ProveReplicaUpdates3Params, SectorUpdateManifest, State, ERR_NOTIFICATION_RECEIVER_ABORTED,
    ERR_NOTIFICATION_REJECTED,
};
use fil_actors_runtime::runtime::Runtime;
use fil_actors_runtime::test_utils::{expect_abort_contains_message, MockRuntime};
use fil_actors_runtime::EPOCHS_IN_DAY;
use util::*;

mod util;

// Tests for ProveReplicaUpdates2 where the request fails completely

const CLIENT_ID: ActorID = 1000;
const DEFAULT_SECTOR_EXPIRATION_DAYS: ChainEpoch = 220;
const FIRST_SECTOR_NUMBER: SectorNumber = 100;

#[test]
fn reject_unauthorized_caller() {
    let (h, rt, sector_updates) = setup(1, 0, 0, 0);
    let cfg = ProveReplicaUpdatesConfig {
        caller: Some(Address::new_id(CLIENT_ID)),
        ..Default::default()
    };
    expect_abort_contains_message(
        ExitCode::USR_FORBIDDEN,
        "caller",
        h.prove_replica_updates2_batch(&rt, &sector_updates, false, false, cfg),
    );
    h.check_state(&rt);
}

#[test]
fn reject_no_proof_types() {
    let (h, rt, sector_updates) = setup(1, 0, 0, 0);
    let cfg = ProveReplicaUpdatesConfig {
        param_twiddle: Some(Box::new(|p: &mut ProveReplicaUpdates3Params| {
            p.sector_proofs = vec![];
            p.aggregate_proof = RawBytes::default();
        })),
        ..Default::default()
    };
    expect_abort_contains_message(
        ExitCode::USR_ILLEGAL_ARGUMENT,
        "exactly one of sector proofs or aggregate proof must be non-empty",
        h.prove_replica_updates2_batch(&rt, &sector_updates, false, false, cfg),
    );
    h.check_state(&rt);
}

#[test]
fn reject_both_proof_types() {
    let (h, rt, sector_updates) = setup(1, 0, 0, 0);
    let cfg = ProveReplicaUpdatesConfig {
        param_twiddle: Some(Box::new(|p: &mut ProveReplicaUpdates3Params| {
            p.sector_proofs = vec![RawBytes::new(vec![1, 2, 3, 4])];
            p.aggregate_proof = RawBytes::new(vec![1, 2, 3, 4])
        })),
        ..Default::default()
    };
    expect_abort_contains_message(
        ExitCode::USR_ILLEGAL_ARGUMENT,
        "exactly one of sector proofs or aggregate proof must be non-empty",
        h.prove_replica_updates2_batch(&rt, &sector_updates, false, false, cfg),
    );
    h.check_state(&rt);
}

#[test]
fn reject_mismatched_proof_len() {
    let (h, rt, sector_updates) = setup(1, 0, 0, 0);
    let cfg = ProveReplicaUpdatesConfig {
        param_twiddle: Some(Box::new(|p: &mut ProveReplicaUpdates3Params| {
            p.sector_proofs.push(RawBytes::new(vec![1, 2, 3, 4]));
        })),
        ..Default::default()
    };
    expect_abort_contains_message(
        ExitCode::USR_ILLEGAL_ARGUMENT,
        "mismatched lengths",
        h.prove_replica_updates2_batch(&rt, &sector_updates, false, false, cfg),
    );
    h.check_state(&rt);
}

#[test]
fn reject_aggregate_proof() {
    let (h, rt, sector_updates) = setup(1, 0, 0, 0);
    let cfg = ProveReplicaUpdatesConfig {
        param_twiddle: Some(Box::new(|p: &mut ProveReplicaUpdates3Params| {
            p.sector_proofs = vec![];
            p.aggregate_proof = RawBytes::new(vec![1, 2, 3, 4]);
            p.aggregate_proof_type = Some(SnarkPackV2);
        })),
        ..Default::default()
    };
    expect_abort_contains_message(
        ExitCode::USR_ILLEGAL_ARGUMENT,
        "aggregate update proofs not yet supported",
        h.prove_replica_updates2_batch(&rt, &sector_updates, false, false, cfg),
    );
    h.check_state(&rt);
}

#[test]
fn reject_all_proofs_fail() {
    let (h, rt, sector_updates) = setup(2, 0, 0, 0);
    let cfg = ProveReplicaUpdatesConfig { proof_failure: vec![0, 1], ..Default::default() };
    expect_abort_contains_message(
        ExitCode::USR_ILLEGAL_ARGUMENT,
        "no valid updates",
        h.prove_replica_updates2_batch(&rt, &sector_updates, false, false, cfg),
    );
    h.check_state(&rt);
}

#[test]
fn reject_invalid_update() {
    let (h, rt, mut sector_updates) = setup(2, 0, 0, 0);
    sector_updates[1].sector = sector_updates[0].sector;
    let cfg = ProveReplicaUpdatesConfig::default();
    expect_abort_contains_message(
        ExitCode::USR_ILLEGAL_ARGUMENT,
        "invalid update 1 while requiring activation success",
        h.prove_replica_updates2_batch(&rt, &sector_updates, true, false, cfg),
    );
    h.check_state(&rt);
}

#[test]
fn reject_required_proof_failure() {
    let (h, rt, sector_updates) = setup(2, 0, 0, 0);
    let cfg = ProveReplicaUpdatesConfig { proof_failure: vec![0], ..Default::default() };
    expect_abort_contains_message(
        ExitCode::USR_ILLEGAL_ARGUMENT,
        "requiring activation success",
        h.prove_replica_updates2_batch(&rt, &sector_updates, true, false, cfg),
    );
    h.check_state(&rt);
}

#[test]
fn reject_required_claim_failure() {
    let (h, rt, sector_updates) = setup(2, CLIENT_ID, 1, 0);
    let cfg = ProveReplicaUpdatesConfig { claim_failure: vec![0], ..Default::default() };
    expect_abort_contains_message(
        ExitCode::USR_ILLEGAL_ARGUMENT,
        "error claiming allocations",
        h.prove_replica_updates2_batch(&rt, &sector_updates, true, false, cfg),
    );
    h.check_state(&rt);
}

#[test]
fn reject_required_notification_abort() {
    let deal_id = 2000;
    let (h, rt, sector_updates) = setup(1, 0, 0, deal_id);

    let cfg = ProveReplicaUpdatesConfig {
        notification_result: Some(ExitCode::USR_UNSPECIFIED),
        ..Default::default()
    };
    expect_abort_contains_message(
        ERR_NOTIFICATION_RECEIVER_ABORTED,
        "receiver aborted",
        h.prove_replica_updates2_batch(&rt, &sector_updates, false, true, cfg),
    );
    h.check_state(&rt);
}

#[test]
fn reject_required_notification_rejected() {
    let deal_id = 2000;
    let (h, rt, sector_updates) = setup(1, 0, 0, deal_id);

    let cfg = ProveReplicaUpdatesConfig { notification_rejected: true, ..Default::default() };
    expect_abort_contains_message(
        ERR_NOTIFICATION_REJECTED,
        "sector change rejected",
        h.prove_replica_updates2_batch(&rt, &sector_updates, false, true, cfg),
    );
    h.check_state(&rt);
}

fn setup(
    sector_count: usize,
    client: ActorID,
    alloc: AllocationID,
    deal: DealID,
) -> (ActorHarness, MockRuntime, Vec<SectorUpdateManifest>) {
    let h = ActorHarness::new_with_options(HarnessOptions::default());
    let rt = h.new_runtime();
    rt.set_balance(BIG_BALANCE.clone());
    h.construct_and_verify(&rt);

    // Onboard empty sectors.
    let sector_expiry = *rt.epoch.borrow() + DEFAULT_SECTOR_EXPIRATION_DAYS * EPOCHS_IN_DAY;
    let sectors = onboard_empty_sectors(&rt, &h, sector_expiry, FIRST_SECTOR_NUMBER, sector_count);

    let st: State = h.get_state(&rt);
    let store = rt.store();
    let piece_size = h.sector_size as u64;
    let sector_updates = sectors
        .iter()
        .map(|s| {
            make_update_manifest(&st, store, s.sector_number, &[(piece_size, client, alloc, deal)])
        })
        .collect();
    (h, rt, sector_updates)
}
