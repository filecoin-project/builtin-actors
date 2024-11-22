use cid::Cid;
use fvm_ipld_bitfield::BitField;
use fvm_shared::address::Address;
use fvm_shared::bigint::{BigInt, Zero};
use fvm_shared::clock::ChainEpoch;
use fvm_shared::deal::DealID;
use fvm_shared::econ::TokenAmount;
use fvm_shared::error::ExitCode;
use fvm_shared::piece::{PaddedPieceSize, PieceInfo};
use fvm_shared::sector::SectorSize;
use fvm_shared::sector::StoragePower;
use fvm_shared::sector::{RegisteredSealProof, SectorNumber};

use export_macro::vm_test;
use fil_actor_market::Method as MarketMethod;
use fil_actor_market::State as MarketState;
use fil_actor_miner::{
    power_for_sector, DisputeWindowedPoStParams, ExpirationExtension, ExtendSectorExpirationParams,
    Method as MinerMethod, PowerPair, ProveReplicaUpdatesParams, ReplicaUpdate, SectorOnChainInfo,
    SectorOnChainInfoFlags, Sectors, State as MinerState, TerminateSectorsParams,
    TerminationDeclaration, SECTORS_AMT_BITWIDTH,
};
use fil_actor_verifreg::Method as VerifregMethod;
use fil_actors_runtime::runtime::policy_constants::MARKET_DEFAULT_ALLOCATION_TERM_BUFFER;
use fil_actors_runtime::runtime::Policy;
use fil_actors_runtime::test_utils::make_sealed_cid;
use fil_actors_runtime::VERIFIED_REGISTRY_ACTOR_ADDR;
use fil_actors_runtime::{Array, EPOCHS_IN_DAY, STORAGE_MARKET_ACTOR_ADDR};
use vm_api::trace::ExpectInvocation;
use vm_api::util::{apply_code, apply_ok, get_state, mutate_state, DynBlockstore};
use vm_api::VM;

use crate::expects::Expect;

use crate::util::{
    advance_by_deadline_to_epoch, advance_by_deadline_to_index, advance_to_proving_deadline,
    assert_invariants, bf_all, check_sector_active, check_sector_faulty, create_accounts,
    create_miner, cron_tick, deadline_state, declare_recovery, expect_invariants, get_deal_weights,
    get_network_stats, invariant_failure_patterns, make_bitfield, market_publish_deal,
    miner_balance, miner_power, miner_prove_sector, override_compute_unsealed_sector_cid,
    precommit_sectors_v2, prove_commit_sectors, sector_info, submit_invalid_post,
    submit_windowed_post, verifreg_add_client, verifreg_add_verifier,
};

#[vm_test]
pub fn replica_update_full_path_success_test(v: &dyn VM) {
    let policy = Policy::default();
    let (sector_info, worker, miner_id, deadline_index, partition_index, sector_size) =
        create_miner_and_upgrade_sector(v);

    let sector_number = sector_info.sector_number;

    // submit post successfully
    let (mut deadline_info, _) = advance_to_proving_deadline(v, &miner_id, sector_number);
    submit_windowed_post(
        v,
        &worker,
        &miner_id,
        deadline_info,
        partition_index,
        Some(PowerPair::zero()),
    );

    // move out of the sector's deadline
    advance_by_deadline_to_index(v, &miner_id, deadline_index + 1 % policy.wpost_period_deadlines);
    assert!(check_sector_active(v, &miner_id, sector_number));

    // miss next post, lose power, become faulty :'(
    advance_by_deadline_to_index(v, &miner_id, deadline_index);
    advance_by_deadline_to_index(v, &miner_id, deadline_index + 1 % policy.wpost_period_deadlines);
    assert!(!check_sector_active(v, &miner_id, sector_number));
    assert!(check_sector_faulty(v, &miner_id, deadline_index, partition_index, sector_number));

    assert!(miner_power(v, &miner_id).is_zero());

    declare_recovery(v, &worker, &miner_id, deadline_index, partition_index, sector_number);
    (deadline_info, _) = advance_to_proving_deadline(v, &miner_id, sector_number);

    submit_windowed_post(
        v,
        &worker,
        &miner_id,
        deadline_info,
        partition_index,
        Some(PowerPair {
            raw: StoragePower::from(sector_size as i64),
            qa: StoragePower::from(sector_size as i64),
        }),
    );

    assert!(check_sector_active(v, &miner_id, sector_number));
    assert!(!check_sector_faulty(v, &miner_id, deadline_index, partition_index, sector_number));
    assert_eq!(miner_power(v, &miner_id).raw, BigInt::from(sector_size as i64));

    assert_invariants(v, &Policy::default(), None)
}

#[vm_test]
pub fn replica_update_full_path_success(v: &dyn VM) {
    replica_update_full_path_success_test(v);
}

#[vm_test]
pub fn upgrade_and_miss_post_test(v: &dyn VM) {
    let (sector_info, worker, miner_id, deadline_index, partition_index, sector_size) =
        create_miner_and_upgrade_sector(v);
    let sector_number = sector_info.sector_number;
    let policy = Policy::default();

    let power_after_update = miner_power(v, &miner_id);
    assert!(!power_after_update.is_zero());

    // immediately miss post, lose power, become faulty
    advance_by_deadline_to_index(v, &miner_id, deadline_index);
    advance_by_deadline_to_index(v, &miner_id, deadline_index + 1 % policy.wpost_period_deadlines);
    assert!(!check_sector_active(v, &miner_id, sector_number));
    assert!(check_sector_faulty(v, &miner_id, deadline_index, partition_index, sector_number));

    let deadline_state = deadline_state(v, &miner_id, deadline_index);
    assert_eq!(power_after_update, deadline_state.faulty_power);

    let empty_sectors_array = Array::<(), _>::new_with_bit_width(
        &DynBlockstore::wrap(v.blockstore()),
        SECTORS_AMT_BITWIDTH,
    )
    .flush()
    .unwrap();
    assert_eq!(deadline_state.sectors_snapshot, empty_sectors_array);

    assert!(miner_power(v, &miner_id).is_zero());

    declare_recovery(v, &worker, &miner_id, deadline_index, partition_index, sector_number);
    let (deadline_info, _) = advance_to_proving_deadline(v, &miner_id, sector_number);

    submit_windowed_post(
        v,
        &worker,
        &miner_id,
        deadline_info,
        partition_index,
        Some(PowerPair {
            raw: StoragePower::from(sector_size as i64),
            qa: StoragePower::from(sector_size as i64),
        }),
    );

    assert!(check_sector_active(v, &miner_id, sector_number));
    assert!(!check_sector_faulty(v, &miner_id, deadline_index, partition_index, sector_number));
    assert_eq!(miner_power(v, &miner_id).raw, BigInt::from(sector_size as i64));

    assert_invariants(v, &Policy::default(), None)
}

#[vm_test]
pub fn prove_replica_update_multi_dline_test(v: &dyn VM) {
    let policy = Policy::default();
    let addrs = create_accounts(v, 1, &TokenAmount::from_whole(1_000_000));
    let (worker, owner) = (addrs[0], addrs[0]);
    let seal_proof = RegisteredSealProof::StackedDRG32GiBV1P1;
    let (maddr, _) = create_miner(
        v,
        &owner,
        &worker,
        seal_proof.registered_window_post_proof().unwrap(),
        &TokenAmount::from_whole(100_000),
    );

    v.set_epoch(1440);
    // something offset far away from deadline 0 and 1

    /* Commit enough sectors to pack two partitions */
    let more_than_one_partition = 2440;
    let batch_size = 100;
    let first_sector_number_p1 = 0;
    let first_sector_number_p2 = seal_proof.window_post_partitions_sector().unwrap();
    let expiration = v.epoch() + policy.max_sector_expiration_extension;

    let new_precommits = precommit_sectors_v2(
        v,
        more_than_one_partition,
        batch_size,
        vec![],
        &worker,
        &maddr,
        seal_proof,
        first_sector_number_p1,
        true,
        Some(expiration),
    );

    let precommits = new_precommits;
    let to_prove = precommits;

    let prove_time = v.epoch() + policy.pre_commit_challenge_delay + 1;
    advance_by_deadline_to_epoch(v, &maddr, prove_time);

    prove_commit_sectors(v, &worker, &maddr, to_prove, batch_size);

    /* This is a mess, but it just ensures activation of both partitions by posting, cronning and checking */

    // advance to proving period and submit post for first partition
    let (deadline_info, partition_index) =
        advance_to_proving_deadline(v, &maddr, first_sector_number_p1);

    // first partition shouldn't be active until PoSt
    assert!(!check_sector_active(v, &maddr, deadline_info.index));
    submit_windowed_post(v, &worker, &maddr, deadline_info, partition_index, None);

    // move into the next deadline so that the first batch of created sectors are active
    let current_deadline_info = advance_by_deadline_to_index(
        v,
        &maddr,
        deadline_info.index + 1 % policy.wpost_period_deadlines,
    );

    // hooray, first partition is now active
    assert_eq!(1, current_deadline_info.index);
    assert!(check_sector_active(v, &maddr, first_sector_number_p1));
    assert!(check_sector_active(v, &maddr, first_sector_number_p1 + 1));
    assert!(check_sector_active(v, &maddr, first_sector_number_p1 + 2));
    assert!(check_sector_active(v, &maddr, first_sector_number_p1 + 2300));

    // second partition shouldn't be active until PoSt
    assert!(!check_sector_active(v, &maddr, first_sector_number_p2));
    submit_windowed_post(v, &worker, &maddr, current_deadline_info, 0, None);

    // move into the next deadline so that the second batch of created sectors are active
    advance_by_deadline_to_index(
        v,
        &maddr,
        deadline_info.index + 2 % policy.wpost_period_deadlines,
    );
    assert!(check_sector_active(v, &maddr, first_sector_number_p2));

    /* Replica Update across two deadlines */
    let old_sector_commr_p1 = sector_info(v, &maddr, first_sector_number_p1).sealed_cid;
    let old_sector_commr_p2 = sector_info(v, &maddr, first_sector_number_p2).sealed_cid;

    let deal_ids = create_deals(2, v, worker, worker, maddr);

    let new_sealed_cid1 = make_sealed_cid(b"replica1");
    let replica_update_1 = ReplicaUpdate {
        sector_number: first_sector_number_p1,
        deadline: 0,
        partition: 0,
        new_sealed_cid: new_sealed_cid1,
        deals: deal_ids[0..1].to_vec(),
        update_proof_type: fvm_shared::sector::RegisteredUpdateProof::StackedDRG32GiBV1,
        replica_proof: vec![].into(),
    };

    let new_sealed_cid2 = make_sealed_cid(b"replica2");
    let replica_update_2 = ReplicaUpdate {
        sector_number: first_sector_number_p2,
        deadline: 1,
        partition: 0,
        new_sealed_cid: new_sealed_cid2,
        deals: deal_ids[1..].to_vec(),
        update_proof_type: fvm_shared::sector::RegisteredUpdateProof::StackedDRG32GiBV1,
        replica_proof: vec![].into(),
    };

    let ret_bf: BitField = apply_ok(
        v,
        &worker,
        &maddr,
        &TokenAmount::zero(),
        MinerMethod::ProveReplicaUpdates as u64,
        Some(ProveReplicaUpdatesParams { updates: vec![replica_update_1, replica_update_2] }),
    )
    .deserialize()
    .unwrap();

    assert_eq!(ret_bf.len(), 2);
    assert!(ret_bf.get(first_sector_number_p1));
    assert!(ret_bf.get(first_sector_number_p2));

    let new_sector_info_p1 = sector_info(v, &maddr, first_sector_number_p1);
    let duration = new_sector_info_p1.expiration - new_sector_info_p1.power_base_epoch;
    let deal_weights1 = get_deal_weights(v, deal_ids[0], duration);
    assert_eq!(deal_weights1.0, new_sector_info_p1.deal_weight);
    assert_eq!(deal_weights1.1, new_sector_info_p1.verified_deal_weight);
    assert_eq!(old_sector_commr_p1, new_sector_info_p1.sector_key_cid.unwrap());
    assert_eq!(new_sealed_cid1, new_sector_info_p1.sealed_cid);
    let new_sector_info_p2 = sector_info(v, &maddr, first_sector_number_p2);
    let deal_weights2 = get_deal_weights(v, deal_ids[1], duration);
    assert_eq!(deal_weights2.0, new_sector_info_p2.deal_weight);
    assert_eq!(deal_weights2.1, new_sector_info_p2.verified_deal_weight);
    assert_eq!(old_sector_commr_p2, new_sector_info_p2.sector_key_cid.unwrap());
    assert_eq!(new_sealed_cid2, new_sector_info_p2.sealed_cid);

    assert_invariants(v, &Policy::default(), None);
}

// ---- Failure cases ----

/// Tests that a sector in an immutable deadline cannot be upgraded
#[vm_test]
pub fn immutable_deadline_failure_test(v: &dyn VM) {
    let addrs = create_accounts(v, 1, &TokenAmount::from_whole(100_000));
    let (worker, owner) = (addrs[0], addrs[0]);
    let seal_proof = RegisteredSealProof::StackedDRG32GiBV1P1;
    let (maddr, robust) = create_miner(
        v,
        &owner,
        &worker,
        seal_proof.registered_window_post_proof().unwrap(),
        &TokenAmount::from_whole(10_000),
    );

    // advance to have seal randomness epoch in the past
    v.set_epoch(200);

    let sector_number = 100;
    let (d_idx, p_idx) = create_sector(v, worker, maddr, sector_number, seal_proof);

    // make some deals
    let deal_ids = create_deals(1, v, worker, worker, maddr);

    // Advance back into the sector's deadline
    advance_to_proving_deadline(v, &maddr, sector_number);

    // replicaUpdate the sector
    let new_cid = make_sealed_cid(b"replica1");
    let replica_update = ReplicaUpdate {
        sector_number,
        deadline: d_idx,
        partition: p_idx,
        new_sealed_cid: new_cid,
        deals: deal_ids,
        update_proof_type: fvm_shared::sector::RegisteredUpdateProof::StackedDRG32GiBV1,
        replica_proof: vec![].into(),
    };
    apply_code(
        v,
        &worker,
        &robust,
        &TokenAmount::zero(),
        MinerMethod::ProveReplicaUpdates as u64,
        Some(ProveReplicaUpdatesParams { updates: vec![replica_update] }),
        ExitCode::USR_ILLEGAL_ARGUMENT,
    );

    assert_invariants(v, &Policy::default(), None)
}

#[vm_test]
pub fn unhealthy_sector_failure_test(v: &dyn VM) {
    let policy = Policy::default();
    let addrs = create_accounts(v, 1, &TokenAmount::from_whole(100_000));
    let (worker, owner) = (addrs[0], addrs[0]);
    let seal_proof = RegisteredSealProof::StackedDRG32GiBV1P1;
    let (maddr, robust) = create_miner(
        v,
        &owner,
        &worker,
        seal_proof.registered_window_post_proof().unwrap(),
        &TokenAmount::from_whole(10_000),
    );

    // advance to have seal randomness epoch in the past
    v.set_epoch(200);

    let sector_number = 100;
    let (d_idx, p_idx) = create_sector(v, worker, maddr, sector_number, seal_proof);

    // make some deals
    let deal_ids = create_deals(1, v, worker, worker, maddr);

    // ffw 2 days, missing posts
    let two_days_later = v.epoch() + policy.wpost_proving_period * 2;
    advance_by_deadline_to_epoch(v, &maddr, two_days_later);
    assert!(!check_sector_active(v, &maddr, sector_number));
    assert!(check_sector_faulty(v, &maddr, d_idx, p_idx, sector_number));

    // replicaUpdate the sector
    let new_cid = make_sealed_cid(b"replica1");
    let replica_update = ReplicaUpdate {
        sector_number,
        deadline: d_idx,
        partition: p_idx,
        new_sealed_cid: new_cid,
        deals: deal_ids,
        update_proof_type: fvm_shared::sector::RegisteredUpdateProof::StackedDRG32GiBV1,
        replica_proof: vec![].into(),
    };
    apply_code(
        v,
        &worker,
        &robust,
        &TokenAmount::zero(),
        MinerMethod::ProveReplicaUpdates as u64,
        Some(ProveReplicaUpdatesParams { updates: vec![replica_update] }),
        ExitCode::USR_ILLEGAL_ARGUMENT,
    );

    expect_invariants(
        v,
        &Policy::default(),
        &[invariant_failure_patterns::REWARD_STATE_EPOCH_MISMATCH.to_owned()],
        None,
    );
}

#[vm_test]
pub fn terminated_sector_failure_test(v: &dyn VM) {
    let addrs = create_accounts(v, 1, &TokenAmount::from_whole(100_000));
    let (worker, owner) = (addrs[0], addrs[0]);
    let seal_proof = RegisteredSealProof::StackedDRG32GiBV1P1;
    let (maddr, robust) = create_miner(
        v,
        &owner,
        &worker,
        seal_proof.registered_window_post_proof().unwrap(),
        &TokenAmount::from_whole(10_000),
    );

    // advance to have seal randomness epoch in the past
    v.set_epoch(200);

    let sector_number = 100;
    let (d_idx, p_idx) = create_sector(v, worker, maddr, sector_number, seal_proof);

    // make some deals
    let deal_ids = create_deals(1, v, worker, worker, maddr);

    // terminate sector

    let terminate_parms = TerminateSectorsParams {
        terminations: vec![TerminationDeclaration {
            deadline: d_idx,
            partition: p_idx,
            sectors: make_bitfield(&[sector_number]),
        }],
    };
    apply_ok(
        v,
        &worker,
        &maddr,
        &TokenAmount::zero(),
        MinerMethod::TerminateSectors as u64,
        Some(terminate_parms),
    );

    // replicaUpdate the sector
    let new_cid = make_sealed_cid(b"replica1");
    let replica_update = ReplicaUpdate {
        sector_number,
        deadline: d_idx,
        partition: p_idx,
        new_sealed_cid: new_cid,
        deals: deal_ids,
        update_proof_type: fvm_shared::sector::RegisteredUpdateProof::StackedDRG32GiBV1,
        replica_proof: vec![].into(),
    };
    apply_code(
        v,
        &worker,
        &robust,
        &TokenAmount::zero(),
        MinerMethod::ProveReplicaUpdates as u64,
        Some(ProveReplicaUpdatesParams { updates: vec![replica_update] }),
        ExitCode::USR_ILLEGAL_ARGUMENT,
    );

    assert_invariants(v, &Policy::default(), None)
}

#[vm_test]
pub fn bad_batch_size_failure_test(v: &dyn VM) {
    let policy = Policy::default();
    let addrs = create_accounts(v, 1, &TokenAmount::from_whole(100_000));
    let (worker, owner) = (addrs[0], addrs[0]);
    let seal_proof = RegisteredSealProof::StackedDRG32GiBV1P1;
    let (maddr, robust) = create_miner(
        v,
        &owner,
        &worker,
        seal_proof.registered_window_post_proof().unwrap(),
        &TokenAmount::from_whole(10_000),
    );

    // advance to have seal randomness epoch in the past
    v.set_epoch(200);

    let sector_number = 100;
    let (d_idx, p_idx) = create_sector(v, worker, maddr, sector_number, seal_proof);

    // make some deals
    let deal_ids = create_deals(1, v, worker, worker, maddr);

    // fail to replicaUpdate more sectors than batch size
    let new_cid = make_sealed_cid(b"replica1");
    let mut updates = vec![];

    for _ in 0..policy.prove_replica_updates_max_size + 1 {
        updates.push(ReplicaUpdate {
            sector_number,
            deadline: d_idx,
            partition: p_idx,
            new_sealed_cid: new_cid,
            deals: deal_ids.clone(),
            update_proof_type: fvm_shared::sector::RegisteredUpdateProof::StackedDRG32GiBV1,
            replica_proof: vec![].into(),
        });
    }

    apply_code(
        v,
        &worker,
        &robust,
        &TokenAmount::zero(),
        MinerMethod::ProveReplicaUpdates as u64,
        Some(ProveReplicaUpdatesParams { updates }),
        ExitCode::USR_ILLEGAL_ARGUMENT,
    );

    assert_invariants(v, &Policy::default(), None)
}

#[vm_test]
pub fn nodispute_after_upgrade_test(v: &dyn VM) {
    let (_, worker, miner_id, deadline_index, _, _) = create_miner_and_upgrade_sector(v);

    let dispute_params = DisputeWindowedPoStParams { deadline: deadline_index, post_index: 0 };
    apply_code(
        v,
        &worker,
        &miner_id,
        &TokenAmount::zero(),
        MinerMethod::DisputeWindowedPoSt as u64,
        Some(dispute_params),
        ExitCode::USR_ILLEGAL_ARGUMENT,
    );

    assert_invariants(v, &Policy::default(), None)
}

#[vm_test]
pub fn upgrade_bad_post_dispute_test(v: &dyn VM) {
    let (sector_info, worker, miner_id, deadline_index, partition_index, _) =
        create_miner_and_upgrade_sector(v);
    let policy = Policy::default();
    let sector_number = sector_info.sector_number;

    let (deadline_info, _) = advance_to_proving_deadline(v, &miner_id, sector_number);
    submit_invalid_post(v, &worker, &miner_id, deadline_info, partition_index);

    advance_by_deadline_to_index(v, &miner_id, deadline_index + 2 % policy.wpost_period_deadlines);

    let dispute_params = DisputeWindowedPoStParams { deadline: deadline_index, post_index: 0 };
    apply_ok(
        v,
        &worker,
        &miner_id,
        &TokenAmount::zero(),
        MinerMethod::DisputeWindowedPoSt as u64,
        Some(dispute_params),
    );

    assert_invariants(v, &Policy::default(), None)
}

#[vm_test]
pub fn bad_post_upgrade_dispute_test(v: &dyn VM) {
    let policy = Policy::default();
    let addrs = create_accounts(v, 1, &TokenAmount::from_whole(100_000));
    let (worker, owner) = (addrs[0], addrs[0]);
    let seal_proof = RegisteredSealProof::StackedDRG32GiBV1P1;
    let (maddr, robust) = create_miner(
        v,
        &owner,
        &worker,
        seal_proof.registered_window_post_proof().unwrap(),
        &TokenAmount::from_whole(10_000),
    );

    // advance to have seal randomness epoch in the past
    v.set_epoch(200);

    let sector_number = 100;
    let (d_idx, p_idx) = create_sector(v, worker, maddr, sector_number, seal_proof);
    let old_sector_info = sector_info(v, &maddr, sector_number);

    // submit an invalid post
    let (deadline_info, _) = advance_to_proving_deadline(v, &maddr, sector_number);

    submit_invalid_post(v, &worker, &maddr, deadline_info, p_idx);
    advance_by_deadline_to_index(v, &maddr, d_idx + 2 % policy.wpost_period_deadlines);

    // make some deals
    let deal_ids = create_deals(1, v, worker, worker, maddr);

    // replicaUpdate the sector -- it succeeds
    let new_cid = make_sealed_cid(b"replica1");
    let replica_update = ReplicaUpdate {
        sector_number,
        deadline: d_idx,
        partition: p_idx,
        new_sealed_cid: new_cid,
        deals: deal_ids.clone(),
        update_proof_type: fvm_shared::sector::RegisteredUpdateProof::StackedDRG32GiBV1,
        replica_proof: vec![].into(),
    };

    let updated_sectors: BitField = apply_ok(
        v,
        &worker,
        &robust,
        &TokenAmount::zero(),
        MinerMethod::ProveReplicaUpdates as u64,
        Some(ProveReplicaUpdatesParams { updates: vec![replica_update] }),
    )
    .deserialize()
    .unwrap();
    assert_eq!(vec![100], bf_all(updated_sectors));

    // sanity check the sector after update
    let new_sector_info = sector_info(v, &maddr, sector_number);
    let duration = new_sector_info.expiration - new_sector_info.power_base_epoch;
    let weights = get_deal_weights(v, deal_ids[0], duration);
    assert_eq!(weights.0, new_sector_info.deal_weight);
    assert_eq!(weights.1, new_sector_info.verified_deal_weight);
    assert_eq!(old_sector_info.sealed_cid, new_sector_info.sector_key_cid.unwrap());
    assert_eq!(new_cid, new_sector_info.sealed_cid);

    // BUT, I can still dispute your evil PoSt

    let dispute_params = DisputeWindowedPoStParams { deadline: d_idx, post_index: 0 };
    apply_ok(
        v,
        &worker,
        &maddr,
        &TokenAmount::zero(),
        MinerMethod::DisputeWindowedPoSt as u64,
        Some(dispute_params),
    );

    assert_invariants(v, &Policy::default(), None)
}

/// Tests that an active CC sector can be correctly upgraded, and then the sector can be terminated
#[vm_test]
pub fn terminate_after_upgrade_test(v: &dyn VM) {
    let (sector_info, worker, miner_id, deadline_index, partition_index, _) =
        create_miner_and_upgrade_sector(v);
    let sector_number = sector_info.sector_number;

    let terminate_params = TerminateSectorsParams {
        terminations: vec![TerminationDeclaration {
            deadline: deadline_index,
            partition: partition_index,
            sectors: make_bitfield(&[sector_number]),
        }],
    };

    apply_ok(
        v,
        &worker,
        &miner_id,
        &TokenAmount::zero(),
        MinerMethod::TerminateSectors as u64,
        Some(terminate_params),
    );

    // expect power, market and miner to be in base state
    let miner_balances = miner_balance(v, &miner_id);
    assert!(miner_balances.initial_pledge.is_zero());
    assert!(miner_balances.pre_commit_deposit.is_zero());

    let network_stats = get_network_stats(v);
    assert!(network_stats.miner_above_min_power_count.is_zero());
    assert!(network_stats.total_raw_byte_power.is_zero());
    assert!(network_stats.total_quality_adj_power.is_zero());
    assert!(network_stats.total_bytes_committed.is_zero());
    assert!(network_stats.total_qa_bytes_committed.is_zero());
    assert!(network_stats.total_pledge_collateral.is_zero());

    assert_invariants(v, &Policy::default(), None);
}

/// Tests that an active CC sector can be correctly upgraded, and then the sector can be extended
#[vm_test]
pub fn extend_after_upgrade_test(v: &dyn VM) {
    let policy = Policy::default();
    let (sector_info, worker, miner_id, deadline_index, partition_index, _) =
        create_miner_and_upgrade_sector(v);

    let sector_number = sector_info.sector_number;
    let mut legacy_sector = sector_info;
    legacy_sector.flags.set(SectorOnChainInfoFlags::SIMPLE_QA_POWER, false);

    let blockstore = &DynBlockstore::wrap(v.blockstore());
    mutate_state(v, &miner_id, |st: &mut MinerState| {
        let mut sectors = Sectors::load(&blockstore, &st.sectors).unwrap();
        sectors.store(vec![legacy_sector]).unwrap();
        st.sectors = sectors.amt.flush().unwrap();
    });

    let extension_epoch = v.epoch();
    let extension_params = ExtendSectorExpirationParams {
        extensions: vec![ExpirationExtension {
            deadline: deadline_index,
            partition: partition_index,
            sectors: make_bitfield(&[sector_number]),
            new_expiration: extension_epoch + policy.max_sector_expiration_extension - 1,
        }],
    };

    apply_ok(
        v,
        &worker,
        &miner_id,
        &TokenAmount::zero(),
        MinerMethod::ExtendSectorExpiration as u64,
        Some(extension_params),
    );

    let miner_state: MinerState = get_state(v, &miner_id).unwrap();
    let final_sector_info = miner_state
        .get_sector(&DynBlockstore::wrap(v.blockstore()), sector_number)
        .unwrap()
        .unwrap();
    assert_eq!(
        policy.max_sector_expiration_extension - 1,
        final_sector_info.expiration - extension_epoch,
    );

    assert_invariants(v, &Policy::default(), None)
}

#[vm_test]
pub fn wrong_deadline_index_failure_test(v: &dyn VM) {
    let policy = Policy::default();
    let addrs = create_accounts(v, 1, &TokenAmount::from_whole(100_000));
    let (worker, owner) = (addrs[0], addrs[0]);
    let seal_proof = RegisteredSealProof::StackedDRG32GiBV1P1;
    let (maddr, robust) = create_miner(
        v,
        &owner,
        &worker,
        seal_proof.registered_window_post_proof().unwrap(),
        &TokenAmount::from_whole(10_000),
    );

    // advance to have seal randomness epoch in the past
    v.set_epoch(200);

    let sector_number = 100;
    let (d_idx, p_idx) = create_sector(v, worker, maddr, sector_number, seal_proof);
    let old_sector_info = sector_info(v, &maddr, sector_number);

    // make some deals
    let deal_ids = create_deals(1, v, worker, worker, maddr);

    // fail to replicaUpdate more sectors than batch size
    let new_cid = make_sealed_cid(b"replica1");
    let mut updates = vec![];

    for _ in 0..policy.prove_replica_updates_max_size + 1 {
        updates.push(ReplicaUpdate {
            sector_number,
            deadline: d_idx + 1,
            partition: p_idx,
            new_sealed_cid: new_cid,
            deals: deal_ids.clone(),
            update_proof_type: fvm_shared::sector::RegisteredUpdateProof::StackedDRG32GiBV1,
            replica_proof: vec![].into(),
        });
    }

    apply_code(
        v,
        &worker,
        &robust,
        &TokenAmount::zero(),
        MinerMethod::ProveReplicaUpdates as u64,
        Some(ProveReplicaUpdatesParams { updates }),
        ExitCode::USR_ILLEGAL_ARGUMENT,
    );

    let new_sector_info = sector_info(v, &maddr, sector_number);
    assert_eq!(old_sector_info, new_sector_info);

    assert_invariants(v, &Policy::default(), None)
}

#[vm_test]
pub fn wrong_partition_index_failure_test(v: &dyn VM) {
    let policy = Policy::default();
    let addrs = create_accounts(v, 1, &TokenAmount::from_whole(100_000));
    let (worker, owner) = (addrs[0], addrs[0]);
    let seal_proof = RegisteredSealProof::StackedDRG32GiBV1P1;
    let (maddr, robust) = create_miner(
        v,
        &owner,
        &worker,
        seal_proof.registered_window_post_proof().unwrap(),
        &TokenAmount::from_whole(10_000),
    );

    // advance to have seal randomness epoch in the past
    v.set_epoch(200);

    let sector_number = 100;
    let (d_idx, p_idx) = create_sector(v, worker, maddr, sector_number, seal_proof);
    let old_sector_info = sector_info(v, &maddr, sector_number);

    // make some deals
    let deal_ids = create_deals(1, v, worker, worker, maddr);

    // fail to replicaUpdate more sectors than batch size
    let new_cid = make_sealed_cid(b"replica1");
    let mut updates = vec![];

    for _ in 0..policy.prove_replica_updates_max_size + 1 {
        updates.push(ReplicaUpdate {
            sector_number,
            deadline: d_idx,
            partition: p_idx + 1,
            new_sealed_cid: new_cid,
            deals: deal_ids.clone(),
            update_proof_type: fvm_shared::sector::RegisteredUpdateProof::StackedDRG32GiBV1,
            replica_proof: vec![].into(),
        });
    }

    apply_code(
        v,
        &worker,
        &robust,
        &TokenAmount::zero(),
        MinerMethod::ProveReplicaUpdates as u64,
        Some(ProveReplicaUpdatesParams { updates }),
        ExitCode::USR_ILLEGAL_ARGUMENT,
    );

    let new_sector_info = sector_info(v, &maddr, sector_number);
    assert_eq!(old_sector_info, new_sector_info);

    assert_invariants(v, &Policy::default(), None)
}

#[vm_test]
pub fn deal_included_in_multiple_sectors_failure_test(v: &dyn VM) {
    let policy = Policy::default();
    let addrs = create_accounts(v, 1, &TokenAmount::from_whole(100_000));
    let (worker, owner) = (addrs[0], addrs[0]);
    let seal_proof = RegisteredSealProof::StackedDRG32GiBV1P1;
    let (maddr, _) = create_miner(
        v,
        &owner,
        &worker,
        seal_proof.registered_window_post_proof().unwrap(),
        &TokenAmount::from_whole(10_000),
    );

    //
    // preCommit two sectors
    //
    //
    let first_sector_number = 100;
    let precommits = precommit_sectors_v2(
        v,
        policy.min_aggregated_sectors as usize,
        256,
        vec![],
        &worker,
        &maddr,
        seal_proof,
        first_sector_number,
        true,
        None,
    );

    assert_eq!(policy.min_aggregated_sectors, precommits.len() as u64);

    let miner_balance = miner_balance(v, &maddr);
    assert!(miner_balance.pre_commit_deposit.is_positive());

    let prove_time = v.epoch() + policy.pre_commit_challenge_delay + 1;
    advance_by_deadline_to_epoch(v, &maddr, prove_time);

    prove_commit_sectors(v, &worker, &maddr, precommits, 100);

    // In the same epoch, trigger cron to validate prove commit
    cron_tick(v);

    // advance to proving period and submit post
    let (deadline_info, partition_index) =
        advance_to_proving_deadline(v, &maddr, first_sector_number);

    // sector shouldn't be active until PoSt
    assert!(!check_sector_active(v, &maddr, first_sector_number));
    assert!(!check_sector_active(v, &maddr, first_sector_number + 1));

    submit_windowed_post(v, &worker, &maddr, deadline_info, partition_index, None);

    // move into the next deadline so that the created sectors are mutable
    advance_by_deadline_to_index(
        v,
        &maddr,
        deadline_info.index + 1 % policy.wpost_period_deadlines,
    );

    // sectors are now active!
    assert!(check_sector_active(v, &maddr, first_sector_number));
    assert!(check_sector_active(v, &maddr, first_sector_number + 1));

    // make some unverified deals
    let deal_ids = create_deals_frac(2, v, worker, worker, maddr, 2, false, 180 * EPOCHS_IN_DAY);

    // replicaUpdate the sector
    let new_sealed_cid1 = make_sealed_cid(b"replica1");
    let replica_update_1 = ReplicaUpdate {
        sector_number: first_sector_number,
        deadline: 0,
        partition: 0,
        new_sealed_cid: new_sealed_cid1,
        deals: deal_ids.clone(),
        update_proof_type: fvm_shared::sector::RegisteredUpdateProof::StackedDRG32GiBV1,
        replica_proof: vec![].into(),
    };

    let new_sealed_cid2 = make_sealed_cid(b"replica2");
    let replica_update_2 = ReplicaUpdate {
        sector_number: first_sector_number + 1,
        deadline: 0,
        partition: 0,
        new_sealed_cid: new_sealed_cid2,
        deals: deal_ids.clone(),
        update_proof_type: fvm_shared::sector::RegisteredUpdateProof::StackedDRG32GiBV1,
        replica_proof: vec![].into(),
    };

    let ret_bf: BitField = apply_ok(
        v,
        &worker,
        &maddr,
        &TokenAmount::zero(),
        MinerMethod::ProveReplicaUpdates as u64,
        Some(ProveReplicaUpdatesParams { updates: vec![replica_update_1, replica_update_2] }),
    )
    .deserialize()
    .unwrap();

    assert_eq!(ret_bf.len(), 1);
    assert!(ret_bf.get(first_sector_number));
    assert!(!ret_bf.get(first_sector_number + 1));

    let new_sector_info_p1 = sector_info(v, &maddr, first_sector_number);
    let duration = new_sector_info_p1.expiration - new_sector_info_p1.power_base_epoch;
    let weights1 = get_deal_weights(v, deal_ids[0], duration);
    let weights2 = get_deal_weights(v, deal_ids[1], duration);
    assert_eq!(weights1.0 + weights2.0, new_sector_info_p1.deal_weight);
    assert_eq!(weights1.1 + weights2.1, new_sector_info_p1.verified_deal_weight);
    assert_eq!(new_sealed_cid1, new_sector_info_p1.sealed_cid);

    let new_sector_info_p2 = sector_info(v, &maddr, first_sector_number + 1);
    assert!(new_sector_info_p2.deal_weight.is_zero());
    assert!(new_sector_info_p2.verified_deal_weight.is_zero());
    assert_ne!(new_sealed_cid2, new_sector_info_p2.sealed_cid);

    assert_invariants(v, &Policy::default(), None)
}

#[vm_test]
pub fn replica_update_verified_deal_test(v: &dyn VM) {
    override_compute_unsealed_sector_cid(v);
    let addrs = create_accounts(v, 3, &TokenAmount::from_whole(100_000));
    let (worker, owner, client, verifier) = (addrs[0], addrs[0], addrs[1], addrs[2]);
    let worker_id = worker.id().unwrap();
    let seal_proof = RegisteredSealProof::StackedDRG32GiBV1P1;
    let policy = Policy::default();
    let (maddr, robust) = create_miner(
        v,
        &owner,
        &worker,
        seal_proof.registered_window_post_proof().unwrap(),
        &TokenAmount::from_whole(10_000),
    );
    let miner_id = maddr.id().unwrap();

    // Get client verified
    let datacap = StoragePower::from(32_u128 << 30);
    verifreg_add_verifier(v, &verifier, datacap.clone());
    verifreg_add_client(v, &verifier, &client, datacap);

    // advance to have seal randomness epoch in the past
    v.set_epoch(200);

    let sector_number = 100;
    let (d_idx, p_idx) = create_sector(v, worker, maddr, sector_number, seal_proof);

    let old_sector_info = sector_info(v, &maddr, sector_number);
    // make some deals, chop off market's alloc term buffer from deal lifetime.  This way term max can
    // line up with sector lifetime AND the deal has buffer room to start a bit later while still fitting in the sector
    let deal_ids = create_verified_deals(
        1,
        v,
        client,
        worker,
        maddr,
        old_sector_info.expiration - v.epoch() - policy.market_default_allocation_term_buffer,
    );

    let st: MarketState = get_state(v, &STORAGE_MARKET_ACTOR_ADDR).unwrap();
    let store = DynBlockstore::wrap(v.blockstore());
    let proposal = st.get_proposal(&store, deal_ids[0]).unwrap();

    // replica update
    let new_sealed_cid = make_sealed_cid(b"replica1");

    let replica_update = ReplicaUpdate {
        sector_number,
        deadline: d_idx,
        partition: p_idx,
        new_sealed_cid,
        deals: deal_ids.clone(),
        update_proof_type: fvm_shared::sector::RegisteredUpdateProof::StackedDRG32GiBV1,
        replica_proof: vec![].into(),
    };
    let updated_sectors: BitField = apply_ok(
        v,
        &worker,
        &robust,
        &TokenAmount::zero(),
        MinerMethod::ProveReplicaUpdates as u64,
        Some(ProveReplicaUpdatesParams { updates: vec![replica_update] }),
    )
    .deserialize()
    .unwrap();
    assert_eq!(vec![100], bf_all(updated_sectors));

    let claim_id = 1_u64;
    let deal_term = proposal.end_epoch - proposal.start_epoch;
    let term_max = deal_term + MARKET_DEFAULT_ALLOCATION_TERM_BUFFER;
    let claim_event = Expect::build_verifreg_claim_event(
        "claim",
        claim_id,
        client.id().unwrap(),
        maddr.id().unwrap(),
        &proposal.piece_cid,
        proposal.piece_size.0,
        deal_term,
        term_max,
        v.epoch(),
        sector_number,
    );
    let old_power = power_for_sector(seal_proof.sector_size().unwrap(), &old_sector_info);

    let pieces: Vec<(Cid, u64)> = vec![(proposal.piece_cid, proposal.piece_size.0)];
    let pis: Vec<PieceInfo> =
        vec![PieceInfo { cid: proposal.piece_cid, size: proposal.piece_size }];
    let unsealed_cid = v.primitives().compute_unsealed_sector_cid(seal_proof, &pis).unwrap();

    // check for the expected subcalls
    ExpectInvocation {
        from: worker_id,
        to: maddr,
        method: MinerMethod::ProveReplicaUpdates as u64,
        subinvocs: Some(vec![
            Expect::market_activate_deals(
                miner_id,
                deal_ids.clone(),
                client.id().unwrap(),
                sector_number,
                old_sector_info.expiration,
                old_sector_info.seal_proof,
                true,
            ),
            ExpectInvocation {
                from: miner_id,
                to: VERIFIED_REGISTRY_ACTOR_ADDR,
                method: VerifregMethod::ClaimAllocations as u64,
                events: Some(vec![claim_event]),
                ..Default::default()
            },
            Expect::reward_this_epoch(miner_id),
            Expect::power_current_total(miner_id),
            Expect::power_update_pledge(miner_id, None),
            Expect::power_update_claim(
                miner_id,
                // sector now fully qap, 10x - x = 9x
                PowerPair { raw: StoragePower::zero(), qa: 9 * old_power.qa },
            ),
        ]),
        events: Some(vec![Expect::build_sector_activation_event(
            "sector-updated",
            miner_id,
            sector_number,
            Some(unsealed_cid),
            &pieces,
        )]),
        ..Default::default()
    }
    .matches(v.take_invocations().last().unwrap());

    // sanity check the sector after update
    let new_sector_info = sector_info(v, &maddr, sector_number);
    let duration = new_sector_info.expiration - new_sector_info.power_base_epoch;
    let weights = get_deal_weights(v, deal_ids[0], duration);
    assert_eq!(weights.0, new_sector_info.deal_weight);
    assert_eq!(weights.1, new_sector_info.verified_deal_weight);
    assert_eq!(old_sector_info.sealed_cid, new_sector_info.sector_key_cid.unwrap());
    assert_eq!(new_sealed_cid, new_sector_info.sealed_cid);
}

#[vm_test]
pub fn replica_update_verified_deal_max_term_violated_test(v: &dyn VM) {
    let addrs = create_accounts(v, 3, &TokenAmount::from_whole(100_000));
    let (worker, owner, client, verifier) = (addrs[0], addrs[0], addrs[1], addrs[2]);
    let seal_proof = RegisteredSealProof::StackedDRG32GiBV1P1;
    let policy = Policy::default();
    let (maddr, robust) = create_miner(
        v,
        &owner,
        &worker,
        seal_proof.registered_window_post_proof().unwrap(),
        &TokenAmount::from_whole(10_000),
    );

    // Get client verified
    let datacap = StoragePower::from(32_u128 << 30);
    verifreg_add_verifier(v, &verifier, datacap.clone());
    verifreg_add_client(v, &verifier, &client, datacap);

    // advance to have seal randomness epoch in the past
    v.set_epoch(200);

    let sector_number = 100;
    let (d_idx, p_idx) = create_sector(v, worker, maddr, sector_number, seal_proof);

    let old_sector_info = sector_info(v, &maddr, sector_number);
    // term max of claim is 1 epoch less than the remaining sector lifetime causing get claims validation failure
    let sector_lifetime = old_sector_info.expiration - v.epoch();
    let deal_ids = create_verified_deals(
        1,
        v,
        client,
        worker,
        maddr,
        sector_lifetime - policy.market_default_allocation_term_buffer - 1,
    );

    // replica update
    let new_sealed_cid = make_sealed_cid(b"replica1");
    let replica_update = ReplicaUpdate {
        sector_number,
        deadline: d_idx,
        partition: p_idx,
        new_sealed_cid,
        deals: deal_ids,
        update_proof_type: fvm_shared::sector::RegisteredUpdateProof::StackedDRG32GiBV1,
        replica_proof: vec![].into(),
    };
    apply_code(
        v,
        &worker,
        &robust,
        &TokenAmount::zero(),
        MinerMethod::ProveReplicaUpdates as u64,
        Some(ProveReplicaUpdatesParams { updates: vec![replica_update] }),
        ExitCode::USR_ILLEGAL_ARGUMENT,
    );
}

// This method produces an active, mutable sector, by:
// - PreCommiting a sector
// - fastforwarding time and ProveCommitting it
// - fastforwarding to its Proving period and PoSting it
// - fastforwarding out of the proving period into a new deadline
// This method assumes that this is a miners first and only sector
pub fn create_sector(
    v: &dyn VM,
    worker: Address,
    maddr: Address,
    sector_number: SectorNumber,
    seal_proof: RegisteredSealProof,
) -> (u64, u64) {
    // precommit
    let exp = v.epoch() + Policy::default().max_sector_expiration_extension;
    let precommits = precommit_sectors_v2(
        v,
        1,
        1,
        vec![],
        &worker,
        &maddr,
        seal_proof,
        sector_number,
        true,
        Some(exp),
    );
    assert_eq!(1, precommits.len());
    assert_eq!(sector_number, precommits[0].info.sector_number);
    let balances = miner_balance(v, &maddr);
    assert!(balances.pre_commit_deposit.is_positive());

    // prove commit
    let prove_time = v.epoch() + Policy::default().pre_commit_challenge_delay + 1;
    advance_by_deadline_to_epoch(v, &maddr, prove_time);
    miner_prove_sector(v, &worker, &maddr, sector_number, vec![]);

    cron_tick(v);

    let (dline_info, p_idx) = advance_to_proving_deadline(v, &maddr, sector_number);
    let d_idx = dline_info.index;
    // not active until post
    assert!(!check_sector_active(v, &maddr, sector_number));
    let m_st: MinerState = get_state(v, &maddr).unwrap();
    let sector =
        m_st.get_sector(&DynBlockstore::wrap(v.blockstore()), sector_number).unwrap().unwrap();
    let sector_power = power_for_sector(seal_proof.sector_size().unwrap(), &sector);
    submit_windowed_post(v, &worker, &maddr, dline_info, p_idx, Some(sector_power));

    // move to next deadline to activate power
    advance_by_deadline_to_index(v, &maddr, d_idx + 1 % Policy::default().wpost_period_deadlines);

    // hooray sector is now active
    assert!(check_sector_active(v, &maddr, sector_number));

    // sanity check the sector
    let old_sector_info = sector_info(v, &maddr, sector_number);
    assert!(old_sector_info.verified_deal_weight.is_zero());
    assert!(old_sector_info.deal_weight.is_zero());
    assert_eq!(None, old_sector_info.sector_key_cid);
    let miner_power = miner_power(v, &maddr);
    assert_eq!(StoragePower::from(seal_proof.sector_size().unwrap() as u64), miner_power.raw);

    (d_idx, p_idx)
}
pub fn create_deals(
    num_deals: u32,
    v: &dyn VM,
    client: Address,
    worker: Address,
    maddr: Address,
) -> Vec<DealID> {
    create_deals_frac(num_deals, v, client, worker, maddr, 1, false, 180 * EPOCHS_IN_DAY)
}

fn create_verified_deals(
    num_deals: u32,
    v: &dyn VM,
    client: Address,
    worker: Address,
    maddr: Address,
    deal_lifetime: ChainEpoch,
) -> Vec<DealID> {
    create_deals_frac(num_deals, v, client, worker, maddr, 1, true, deal_lifetime)
}

#[allow(clippy::too_many_arguments)]
fn create_deals_frac(
    num_deals: u32,
    v: &dyn VM,
    client: Address,
    worker: Address,
    maddr: Address,
    size_frac: u8,
    verified_deal: bool,
    deal_lifetime: ChainEpoch,
) -> Vec<DealID> {
    let collateral = TokenAmount::from_whole(3 * num_deals as i64);
    apply_ok(
        v,
        &client,
        &STORAGE_MARKET_ACTOR_ADDR,
        &collateral,
        MarketMethod::AddBalance as u64,
        Some(client),
    );
    apply_ok(
        v,
        &worker,
        &STORAGE_MARKET_ACTOR_ADDR,
        &collateral,
        MarketMethod::AddBalance as u64,
        Some(maddr),
    );

    let mut ids = Vec::<DealID>::new();
    let deal_start = v.epoch() + Policy::default().pre_commit_challenge_delay + 1;

    for i in 0..num_deals {
        let deals = market_publish_deal(
            v,
            &worker,
            &client,
            &maddr,
            format!("deal-label {}", i),
            PaddedPieceSize((32 << 30) / size_frac as u64),
            verified_deal,
            deal_start,
            deal_lifetime,
        );
        for id in deals.ids.iter() {
            ids.push(*id);
        }
    }
    ids
}

pub fn create_miner_and_upgrade_sector(
    v: &dyn VM,
) -> (SectorOnChainInfo, Address, Address, u64, u64, SectorSize) {
    let addrs = create_accounts(v, 1, &TokenAmount::from_whole(100_000));
    let (worker, owner) = (addrs[0], addrs[0]);
    let seal_proof = RegisteredSealProof::StackedDRG32GiBV1P1;
    let (maddr, robust) = create_miner(
        v,
        &owner,
        &worker,
        seal_proof.registered_window_post_proof().unwrap(),
        &TokenAmount::from_whole(10_000),
    );

    // advance to have seal randomness epoch in the past
    v.set_epoch(200);

    let sector_number = 100;
    let (d_idx, p_idx) = create_sector(v, worker, maddr, sector_number, seal_proof);

    let old_sector_info = sector_info(v, &maddr, sector_number);
    // make some deals
    let deal_ids = create_deals(1, v, worker, worker, maddr);

    // replica update
    let new_sealed_cid = make_sealed_cid(b"replica1");
    let updated_sectors: BitField = {
        let replica_update = ReplicaUpdate {
            sector_number,
            deadline: d_idx,
            partition: p_idx,
            new_sealed_cid,
            deals: deal_ids.clone(),
            update_proof_type: fvm_shared::sector::RegisteredUpdateProof::StackedDRG32GiBV1,
            replica_proof: vec![].into(),
        };
        apply_ok(
            v,
            &worker,
            &robust,
            &TokenAmount::zero(),
            MinerMethod::ProveReplicaUpdates as u64,
            Some(ProveReplicaUpdatesParams { updates: vec![replica_update] }),
        )
    }
    .deserialize()
    .unwrap();
    assert_eq!(vec![100], bf_all(updated_sectors));

    // sanity check the sector after update
    let new_sector_info = sector_info(v, &maddr, sector_number);
    let duration = new_sector_info.expiration - new_sector_info.power_base_epoch;
    let weights = get_deal_weights(v, deal_ids[0], duration);
    assert_eq!(weights.0, new_sector_info.deal_weight);
    assert_eq!(weights.1, new_sector_info.verified_deal_weight);
    assert_eq!(old_sector_info.sealed_cid, new_sector_info.sector_key_cid.unwrap());
    assert_eq!(new_sealed_cid, new_sector_info.sealed_cid);
    (new_sector_info, worker, maddr, d_idx, p_idx, seal_proof.sector_size().unwrap())
}
