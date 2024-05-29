use fvm_ipld_encoding::RawBytes;
use fvm_shared::address::Address;
use fvm_shared::deal::DealID;
use fvm_shared::error::ExitCode;
use fvm_shared::sector::SectorNumber;
use fvm_shared::{bigint::Zero, clock::ChainEpoch, econ::TokenAmount, ActorID};

use fil_actor_miner::ext::verifreg::AllocationID;
use fil_actor_miner::{
    ProveCommitSectors3Params, SectorActivationManifest, ERR_NOTIFICATION_RECEIVER_ABORTED,
    ERR_NOTIFICATION_REJECTED,
};
use fil_actors_runtime::test_utils::{expect_abort_contains_message, MockRuntime};
use fil_actors_runtime::EPOCHS_IN_DAY;
use util::*;

mod util;

// Tests for ProveCommitSectors2 where the request fails completely

const CLIENT_ID: ActorID = 1000;
const DEFAULT_SECTOR_EXPIRATION_DAYS: ChainEpoch = 220;
const FIRST_SECTOR_NUMBER: SectorNumber = 100;

#[test]
fn reject_unauthorized_caller() {
    let (h, rt, activations) = setup_precommits(&[(0, 0, 0)]);
    let cfg = ProveCommitSectors3Config {
        caller: Some(Address::new_id(CLIENT_ID)),
        ..Default::default()
    };
    expect_abort_contains_message(
        ExitCode::USR_FORBIDDEN,
        "caller",
        h.prove_commit_sectors3(&rt, &activations, false, false, false, cfg),
    );
    h.check_state(&rt);
}

#[test]
fn reject_no_proof_types() {
    let (h, rt, activations) = setup_precommits(&[(0, 0, 0)]);
    let cfg = ProveCommitSectors3Config {
        param_twiddle: Some(Box::new(|p: &mut ProveCommitSectors3Params| {
            p.sector_proofs = vec![];
            p.aggregate_proof = RawBytes::default();
        })),
        ..Default::default()
    };
    expect_abort_contains_message(
        ExitCode::USR_ILLEGAL_ARGUMENT,
        "exactly one of sector proofs or aggregate proof must be non-empty",
        h.prove_commit_sectors3(&rt, &activations, false, false, false, cfg),
    );
    h.check_state(&rt);
}

#[test]
fn reject_both_proof_types() {
    let (h, rt, activations) = setup_precommits(&[(0, 0, 0)]);
    let cfg = ProveCommitSectors3Config {
        param_twiddle: Some(Box::new(|p: &mut ProveCommitSectors3Params| {
            p.sector_proofs = vec![RawBytes::new(vec![1, 2, 3, 4])];
            p.aggregate_proof = RawBytes::new(vec![1, 2, 3, 4])
        })),
        ..Default::default()
    };
    expect_abort_contains_message(
        ExitCode::USR_ILLEGAL_ARGUMENT,
        "exactly one of sector proofs or aggregate proof must be non-empty",
        h.prove_commit_sectors3(&rt, &activations, false, false, false, cfg),
    );
    h.check_state(&rt);
}

#[test]
fn reject_mismatched_proof_len() {
    let (h, rt, activations) = setup_precommits(&[(0, 0, 0)]);
    let cfg = ProveCommitSectors3Config {
        param_twiddle: Some(Box::new(|p: &mut ProveCommitSectors3Params| {
            p.sector_proofs.push(RawBytes::new(vec![1, 2, 3, 4]));
        })),
        ..Default::default()
    };
    expect_abort_contains_message(
        ExitCode::USR_ILLEGAL_ARGUMENT,
        "mismatched lengths",
        h.prove_commit_sectors3(&rt, &activations, false, false, false, cfg),
    );
    h.check_state(&rt);
}

#[test]
fn reject_too_soon() {
    let (h, rt, activations) = setup_precommits(&[(0, 0, 0)]);
    let epoch = *rt.epoch.borrow();
    rt.set_epoch(epoch - 2);
    let cfg = ProveCommitSectors3Config::default();
    expect_abort_contains_message(
        ExitCode::USR_FORBIDDEN,
        "too early to prove sector",
        h.prove_commit_sectors3(&rt, &activations, false, false, false, cfg),
    );
    h.check_state(&rt);
}

#[test]
fn reject_expired_precommit() {
    let (h, rt, activations) = setup_precommits(&[(0, 0, 0)]);
    let epoch = *rt.epoch.borrow();
    rt.set_epoch(epoch + 31 * EPOCHS_IN_DAY); // Expired.
    let cfg = ProveCommitSectors3Config::default();
    expect_abort_contains_message(
        ExitCode::USR_ILLEGAL_ARGUMENT,
        "no valid precommits",
        h.prove_commit_sectors3(&rt, &activations, false, false, false, cfg),
    );
    h.check_state(&rt);
}

#[test]
fn reject_precommit_deals() {
    let (h, rt) = setup_basic();

    // Precommit sectors, one with a deal
    let precommit_epoch = *rt.epoch.borrow();
    let sector_expiry = precommit_epoch + DEFAULT_SECTOR_EXPIRATION_DAYS * EPOCHS_IN_DAY;
    let piece_size = h.sector_size as u64;
    let mut precommits = make_fake_precommits(
        &h,
        FIRST_SECTOR_NUMBER,
        precommit_epoch - 1,
        sector_expiry,
        &[&[piece_size], &[piece_size]],
    );
    precommits[0].deal_ids.push(1);
    h.pre_commit_sector_batch_v2(&rt, &precommits, true, &TokenAmount::zero()).unwrap();
    rt.set_epoch(precommit_epoch + rt.policy.pre_commit_challenge_delay + 1);

    let manifests: Vec<SectorActivationManifest> = precommits
        .iter()
        .map(|s| make_activation_manifest(s.sector_number, &[(piece_size, 0, 0, 0)]))
        .collect();

    let cfg = ProveCommitSectors3Config { validation_failure: vec![0], ..Default::default() };
    // Single bad precommit aborts with require_activation_success=true.
    expect_abort_contains_message(
        ExitCode::USR_ILLEGAL_ARGUMENT,
        "invalid pre-commit 0 while requiring activation success",
        h.prove_commit_sectors3(&rt, &manifests, true, false, false, cfg),
    );
    h.check_state(&rt);
}

#[test]
fn reject_all_proofs_fail() {
    let (h, rt, activations) = setup_precommits(&[(0, 0, 0), (0, 0, 0)]);
    let cfg = ProveCommitSectors3Config { proof_failure: vec![0, 1], ..Default::default() };
    // If all proofs fail, no need for require_activation_success=true.
    expect_abort_contains_message(
        ExitCode::USR_ILLEGAL_ARGUMENT,
        "no valid proofs",
        h.prove_commit_sectors3(&rt, &activations, false, false, false, cfg),
    );
    h.check_state(&rt);
}

#[test]
fn reject_aggregate_proof_fails() {
    let (h, rt, activations) = setup_precommits(&[(0, 0, 0); 4]);
    let cfg = ProveCommitSectors3Config { proof_failure: vec![0], ..Default::default() };
    // If aggregate proof fails, no need for require_activation_success=true.
    expect_abort_contains_message(
        ExitCode::USR_ILLEGAL_ARGUMENT,
        "invalid aggregate proof",
        h.prove_commit_sectors3(&rt, &activations, false, false, true, cfg),
    );
    h.check_state(&rt);
}

#[test]
fn reject_required_proof_failure() {
    let (h, rt, activations) = setup_precommits(&[(0, 0, 0); 4]);
    let cfg = ProveCommitSectors3Config { proof_failure: vec![0], ..Default::default() };
    // Single proof failure aborts with require_activation_success=true.
    expect_abort_contains_message(
        ExitCode::USR_ILLEGAL_ARGUMENT,
        "invalid proof for sector 100 while requiring activation success",
        h.prove_commit_sectors3(&rt, &activations, true, false, false, cfg),
    );
    h.check_state(&rt);
}

#[test]
fn reject_mismatched_commd() {
    let (h, rt, mut activations) = setup_precommits(&[(0, 0, 0); 2]);
    // Set wrong CID for first sector.
    activations[0].pieces[0].cid = activations[1].pieces[0].cid;

    let cfg = ProveCommitSectors3Config::default();
    expect_abort_contains_message(
        ExitCode::USR_ILLEGAL_ARGUMENT,
        "unsealed CID does not match pieces",
        h.prove_commit_sectors3(&rt, &activations, false, false, false, cfg),
    );
}

#[test]
fn reject_required_claim_failure() {
    let (h, rt, activations) = setup_precommits(&[(0, 0, 0), (CLIENT_ID, 1, 0)]);
    let cfg = ProveCommitSectors3Config { claim_failure: vec![0], ..Default::default() };
    // Single claim failure aborts with require_activation_success=true.
    expect_abort_contains_message(
        ExitCode::USR_ILLEGAL_ARGUMENT,
        "error claiming allocations",
        h.prove_commit_sectors3(&rt, &activations, true, false, false, cfg),
    );
    h.check_state(&rt);
}

#[test]
fn required_notification_abort() {
    let deal_id = 2000;
    let (h, rt, activations) = setup_precommits(&[(0, 0, deal_id)]);
    let cfg = ProveCommitSectors3Config {
        notification_result: Some(ExitCode::USR_ILLEGAL_ARGUMENT),
        ..Default::default()
    };
    expect_abort_contains_message(
        ERR_NOTIFICATION_RECEIVER_ABORTED,
        "receiver aborted",
        h.prove_commit_sectors3(&rt, &activations, true, true, false, cfg),
    );
    h.check_state(&rt);
}

#[test]
fn require_notification_rejected() {
    let deal_id = 2000;
    let (h, rt, activations) = setup_precommits(&[(0, 0, deal_id)]);
    let cfg = ProveCommitSectors3Config { notification_rejected: true, ..Default::default() };
    // Require notification success.
    expect_abort_contains_message(
        ERR_NOTIFICATION_REJECTED,
        "sector change rejected",
        h.prove_commit_sectors3(&rt, &activations, true, true, false, cfg),
    );
    h.check_state(&rt);
}

fn setup_basic() -> (ActorHarness, MockRuntime) {
    let h = ActorHarness::new_with_options(HarnessOptions::default());
    let rt = h.new_runtime();
    rt.set_balance(BIG_BALANCE.clone());
    h.construct_and_verify(&rt);
    (h, rt)
}

fn setup_precommits(
    confs: &[(ActorID, AllocationID, DealID)],
) -> (ActorHarness, MockRuntime, Vec<SectorActivationManifest>) {
    let (h, rt) = setup_basic();

    // Precommit sectors
    let piece_size = h.sector_size as u64; // All sectors have a single full-size piece.
    let precommit_epoch = *rt.epoch.borrow();
    let sector_expiry = *rt.epoch.borrow() + DEFAULT_SECTOR_EXPIRATION_DAYS * EPOCHS_IN_DAY;
    let one_sector_piece_sizes = &[piece_size] as &[u64];
    let piece_sizes = vec![one_sector_piece_sizes; confs.len()];
    let precommits = make_fake_precommits(
        &h,
        FIRST_SECTOR_NUMBER,
        precommit_epoch - 1,
        sector_expiry,
        &piece_sizes,
    );
    h.pre_commit_sector_batch_v2(&rt, &precommits, true, &TokenAmount::zero()).unwrap();
    rt.set_epoch(precommit_epoch + rt.policy.pre_commit_challenge_delay + 1);

    let manifests = precommits
        .iter()
        .zip(confs)
        .map(|(s, c)| make_activation_manifest(s.sector_number, &[(piece_size, c.0, c.1, c.2)]))
        .collect();
    (h, rt, manifests)
}
