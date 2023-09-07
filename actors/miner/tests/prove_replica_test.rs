use fvm_ipld_blockstore::Blockstore;
use fvm_shared::deal::DealID;
use fvm_shared::sector::{SectorNumber, StoragePower};
use fvm_shared::{bigint::Zero, clock::ChainEpoch, econ::TokenAmount, ActorID};

use fil_actor_miner::ext::verifreg::AllocationID;
use fil_actor_miner::ProveReplicaUpdates2Return;
use fil_actor_miner::{
    CompactCommD, PieceActivationManifest, ProveCommitSectors2Return, SectorActivationManifest,
    SectorOnChainInfo, SectorPreCommitInfo, SectorPreCommitOnChainInfo, SectorUpdateManifest,
    State,
};
use fil_actors_runtime::test_utils::{make_piece_cid, make_sealed_cid, MockRuntime};
use fil_actors_runtime::{runtime::Runtime, BatchReturn, DealWeight, EPOCHS_IN_DAY};
use util::*;

mod util;

const DEFAULT_SECTOR_EXPIRATION_DAYS: ChainEpoch = 220;

#[test]
fn prove_basic_updates() {
    let h = ActorHarness::new_with_options(HarnessOptions::default());
    let rt = h.new_runtime();
    rt.set_balance(BIG_BALANCE.clone());
    let client_id: ActorID = 1000;

    h.construct_and_verify(&rt);

    // Onboard a batch of empty sectors.
    rt.set_epoch(1);
    let sector_expiry = *rt.epoch.borrow() + DEFAULT_SECTOR_EXPIRATION_DAYS * EPOCHS_IN_DAY;
    let first_sector_number = 100;
    let sector_count = 4;
    let sectors = onboard_empty_sectors(&rt, &h, sector_expiry, first_sector_number, sector_count);
    let snos = sectors.iter().map(|s| s.sector_number).collect::<Vec<_>>();

    // Update them in batch, each with a single piece.
    let st: State = h.get_state(&rt);
    let store = rt.store();
    let piece_size = h.sector_size as u64;
    let sector_updates = vec![
        make_update_manifest(&st, store, &sectors[0], &[(piece_size, 0, 0, 0)]), // No alloc or deal
        make_update_manifest(&st, store, &sectors[1], &[(piece_size, client_id, 1000, 0)]), // Just an alloc
        make_update_manifest(&st, store, &sectors[2], &[(piece_size, 0, 0, 2000)]), // Just a deal
        make_update_manifest(&st, store, &sectors[3], &[(piece_size, client_id, 1001, 2001)]), // Alloc and deal
    ];

    let result = h.prove_replica_updates2_batch(&rt, &sector_updates, true, true).unwrap();
    assert_eq!(
        ProveReplicaUpdates2Return { activation_results: BatchReturn::ok(sectors.len() as u32) },
        result
    );

    let duration = sector_expiry - *rt.epoch.borrow();
    let expected_weight = DealWeight::from(piece_size) * duration;
    let raw_power = StoragePower::from(h.sector_size as u64);
    let verified_power = &raw_power * 10;
    let raw_pledge = h.initial_pledge_for_power(&rt, &raw_power);
    let verified_pledge = h.initial_pledge_for_power(&rt, &verified_power);

    // Sector 0: Even though there's no "deal", the data weight is set.
    verify_weights(&rt, &h, snos[0], &expected_weight, &DealWeight::zero(), &raw_pledge);
    // Sector 1: With an allocation, the verified weight is set instead.
    verify_weights(&rt, &h, snos[1], &DealWeight::zero(), &expected_weight, &verified_pledge);
    // Sector 2: Deal weight is set.
    verify_weights(&rt, &h, snos[2], &expected_weight, &DealWeight::zero(), &raw_pledge);
    // Sector 3: Deal doesn't make a difference to verified weight only set.
    verify_weights(&rt, &h, snos[3], &DealWeight::zero(), &expected_weight, &verified_pledge);
}

fn onboard_empty_sectors(
    rt: &MockRuntime,
    h: &ActorHarness,
    expiration: ChainEpoch,
    first_sector_number: SectorNumber,
    count: usize,
) -> Vec<SectorOnChainInfo> {
    let precommit_epoch = *rt.epoch.borrow();

    // Precommit the sectors.
    let precommits =
        make_empty_precommits(h, first_sector_number, precommit_epoch - 1, expiration, count);
    h.pre_commit_sector_batch_v2(rt, &precommits, true, &TokenAmount::zero()).unwrap();
    let precommits: Vec<SectorPreCommitOnChainInfo> =
        precommits.iter().map(|sector| h.get_precommit(rt, sector.sector_number)).collect();

    // Prove the sectors.
    // Use the new ProveCommitSectors2
    rt.set_epoch(precommit_epoch + rt.policy.pre_commit_challenge_delay + 1);
    let sectors: Vec<SectorOnChainInfo> = precommits
        .iter()
        .map(|pc| {
            let sector_activation = make_activation_manifest(pc.info.sector_number, &vec![]);
            h.prove_commit_sectors2(rt, &vec![sector_activation], false, false, false).unwrap();
            h.get_sector(rt, pc.info.sector_number)
        })
        .collect();

    // Window PoST to activate the sectors, a pre-requisite for upgrading.
    h.advance_and_submit_posts(rt, &sectors);
    sectors
}

#[test]
fn prove_commit2_basic() {
    let h = ActorHarness::new_with_options(HarnessOptions::default());
    let rt = h.new_runtime();
    rt.set_balance(BIG_BALANCE.clone());
    let client_id: ActorID = 1000;

    h.construct_and_verify(&rt);

    // Precommit sectors
    let precommit_epoch = *rt.epoch.borrow();
    let first_sector_number = 100;
    let sector_count = 4;
    let sector_expiry = *rt.epoch.borrow() + DEFAULT_SECTOR_EXPIRATION_DAYS * EPOCHS_IN_DAY;
    let precommits = make_fake_commd_precommits(
        &h,
        first_sector_number,
        precommit_epoch - 1,
        sector_expiry,
        sector_count,
    );
    h.pre_commit_sector_batch_v2(&rt, &precommits, true, &TokenAmount::zero()).unwrap();
    let snos: Vec<SectorNumber> =
        precommits.iter().map(|pci: &SectorPreCommitInfo| pci.sector_number).collect();

    // Update them in batch, each with a single piece.
    let piece_size = h.sector_size as u64;
    let sector_activations = vec![
        make_activation_manifest(snos[0], &[(piece_size, 0, 0, 0)]), // No alloc or deal
        make_activation_manifest(snos[1], &[(piece_size, client_id, 1000, 0)]), // Just an alloc
        make_activation_manifest(snos[2], &[(piece_size, 0, 0, 2000)]), // Just a deal
        make_activation_manifest(snos[3], &[(piece_size, client_id, 1001, 2001)]), // Alloc and deal
    ];

    rt.set_epoch(precommit_epoch + rt.policy.pre_commit_challenge_delay + 1);
    let result = h.prove_commit_sectors2(&rt, &sector_activations, true, true, false).unwrap();
    assert_eq!(
        ProveCommitSectors2Return { activation_results: BatchReturn::ok(precommits.len() as u32) },
        result
    );

    let duration = sector_expiry - *rt.epoch.borrow();
    let expected_weight = DealWeight::from(piece_size) * duration;
    let raw_power = StoragePower::from(h.sector_size as u64);
    let verified_power = &raw_power * 10;
    let raw_pledge = h.initial_pledge_for_power(&rt, &raw_power);
    let verified_pledge = h.initial_pledge_for_power(&rt, &verified_power);

    // Sector 0: Even though there's no "deal", the data weight is set.
    verify_weights(&rt, &h, snos[0], &expected_weight, &DealWeight::zero(), &raw_pledge);
    // Sector 1: With an allocation, the verified weight is set instead.
    verify_weights(&rt, &h, snos[1], &DealWeight::zero(), &expected_weight, &verified_pledge);
    // Sector 2: Deal weight is set.
    verify_weights(&rt, &h, snos[2], &expected_weight, &DealWeight::zero(), &raw_pledge);
    // Sector 3: Deal doesn't make a difference to verified weight only set.
    verify_weights(&rt, &h, snos[3], &DealWeight::zero(), &expected_weight, &verified_pledge);
}

pub fn make_empty_precommits(
    h: &ActorHarness,
    first_sector_number: SectorNumber,
    challenge: ChainEpoch,
    expiration: ChainEpoch,
    count: usize,
) -> Vec<SectorPreCommitInfo> {
    (0..count)
        .map(|i| {
            let sector_number = first_sector_number + i as u64;
            h.make_pre_commit_params_v2(
                sector_number,
                challenge,
                expiration,
                vec![],
                CompactCommD::empty(),
            )
        })
        .collect()
}

// Note this matches the faked commD computation in the testing harness
pub fn make_fake_commd_precommits(
    h: &ActorHarness,
    first_sector_number: SectorNumber,
    challenge: ChainEpoch,
    expiration: ChainEpoch,
    count: usize,
) -> Vec<SectorPreCommitInfo> {
    (0..count)
        .map(|i| {
            let sector_number = first_sector_number + i as u64;
            h.make_pre_commit_params_v2(
                sector_number,
                challenge,
                expiration,
                vec![],
                CompactCommD(Some(make_piece_cid(
                    format!("unsealed-{}", sector_number).as_bytes(),
                ))),
            )
        })
        .collect()
}

pub fn make_activation_manifest(
    sector_number: SectorNumber,
    piece_specs: &[(u64, ActorID, AllocationID, DealID)],
) -> SectorActivationManifest {
    let pieces: Vec<PieceActivationManifest> = piece_specs
        .iter()
        .enumerate()
        .map(|(i, (sz, client, alloc, deal))| make_piece_manifest(i, *sz, *client, *alloc, *deal))
        .collect();
    SectorActivationManifest { sector_number, pieces }
}

fn make_update_manifest(
    st: &State,
    store: &impl Blockstore,
    sector: &SectorOnChainInfo,
    piece_specs: &[(u64, ActorID, AllocationID, DealID)],
) -> SectorUpdateManifest {
    let (deadline, partition) = st.find_sector(store, sector.sector_number).unwrap();
    let new_sealed_cid = make_sealed_cid(format!("sealed{}", sector.sector_number).as_bytes());
    let pieces: Vec<PieceActivationManifest> = piece_specs
        .iter()
        .enumerate()
        .map(|(i, (sz, client, alloc, deal))| make_piece_manifest(i, *sz, *client, *alloc, *deal))
        .collect();
    SectorUpdateManifest {
        sector: sector.sector_number,
        deadline,
        partition,
        new_sealed_cid,
        pieces,
    }
}

fn verify_weights(
    rt: &MockRuntime,
    h: &ActorHarness,
    sno: SectorNumber,
    data_weight: &DealWeight,
    verified_weight: &DealWeight,
    pledge: &TokenAmount,
) {
    let s = h.get_sector(rt, sno);
    // Deal IDs are deprecated and never set.
    assert!(s.deal_ids.is_empty());
    assert_eq!(data_weight, &s.deal_weight);
    assert_eq!(verified_weight, &s.verified_deal_weight);
    assert_eq!(pledge, &s.initial_pledge);
}
