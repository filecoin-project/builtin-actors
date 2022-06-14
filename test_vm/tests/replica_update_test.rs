use fil_actor_cron::Method as MethodsCron;
use fil_actor_market::Method as MethodsMarket;
use fil_actor_miner::{
    power_for_sector, DisputeWindowedPoStParams, ExpirationExtension, ExtendSectorExpirationParams,
    Method as MethodsMiner, PowerPair, ProveCommitSectorParams, ProveReplicaUpdatesParams,
    ReplicaUpdate, SectorOnChainInfo, State as MinerState, TerminateSectorsParams,
    TerminationDeclaration, SECTORS_AMT_BITWIDTH,
};

use fil_actors_runtime::test_utils::make_sealed_cid;
use fvm_shared::piece::PaddedPieceSize;

use fil_actors_runtime::runtime::Policy;
use fil_actors_runtime::{
    Array, CRON_ACTOR_ADDR, EPOCHS_IN_DAY, STORAGE_MARKET_ACTOR_ADDR, SYSTEM_ACTOR_ADDR,
};
use fvm_ipld_bitfield::BitField;
use fvm_ipld_blockstore::MemoryBlockstore;
use fvm_ipld_encoding::RawBytes;
use fvm_shared::address::Address;
use fvm_shared::bigint::{BigInt, Zero};
use fvm_shared::deal::DealID;
use fvm_shared::econ::TokenAmount;
use fvm_shared::error::ExitCode;
use fvm_shared::sector::SectorSize;
use fvm_shared::sector::StoragePower;
use fvm_shared::sector::{RegisteredSealProof, SectorNumber};
use num_traits::sign::Signed;
use test_vm::util::{
    advance_by_deadline_to_epoch, advance_by_deadline_to_index, advance_to_proving_deadline,
    apply_code, apply_ok, bf_all, check_sector_active, check_sector_faulty, create_accounts,
    create_miner, deadline_state, declare_recovery, make_bitfield, miner_power, precommit_sectors,
    prove_commit_sectors, publish_deal, sector_info, submit_invalid_post, submit_windowed_post, invariant_failure_patterns,
};
use test_vm::VM;

// ---- Success cases ----

// Tests that an active CC sector can be correctly upgraded, and the expected state changes occur
#[test]
fn replica_update_simple_path_sucess() {
    create_miner_and_upgrade_sector(&MemoryBlockstore::new()).0.assert_state_invariants();
}

// Tests a successful upgrade, followed by the sector going faulty and recovering
#[test]
fn replica_update_full_path_sucess() {
    let store = &MemoryBlockstore::new();
    let policy = Policy::default();
    let (v, sector_info, worker, miner_id, deadline_index, partition_index, sector_size) =
        create_miner_and_upgrade_sector(store);
    let sector_number = sector_info.sector_number;

    // submit post successfully
    let (mut deadline_info, _, mut v) = advance_to_proving_deadline(v, miner_id, sector_number);
    submit_windowed_post(&v, worker, miner_id, deadline_info, partition_index, PowerPair::zero());

    // move out of the sector's deadline
    v = advance_by_deadline_to_index(
        v,
        miner_id,
        deadline_index + 1 % policy.wpost_period_deadlines,
    )
    .0;
    assert!(check_sector_active(&v, miner_id, sector_number));

    // miss next post, lose power, become faulty :'(
    v = advance_by_deadline_to_index(v, miner_id, deadline_index).0;
    v = advance_by_deadline_to_index(
        v,
        miner_id,
        deadline_index + 1 % policy.wpost_period_deadlines,
    )
    .0;
    assert!(!check_sector_active(&v, miner_id, sector_number));
    assert!(check_sector_faulty(&v, miner_id, deadline_index, partition_index, sector_number));

    assert!(miner_power(&v, miner_id).is_zero());

    declare_recovery(&v, worker, miner_id, deadline_index, partition_index, sector_number);
    (deadline_info, _, v) = advance_to_proving_deadline(v, miner_id, sector_number);

    submit_windowed_post(
        &v,
        worker,
        miner_id,
        deadline_info,
        partition_index,
        PowerPair {
            raw: StoragePower::from(sector_size as i64),
            qa: StoragePower::from(sector_size as i64),
        },
    );

    assert!(check_sector_active(&v, miner_id, sector_number));
    assert!(!check_sector_faulty(&v, miner_id, deadline_index, partition_index, sector_number));
    assert_eq!(miner_power(&v, miner_id).raw, BigInt::from(sector_size as i64));
    v.assert_state_invariants();
}

#[test]
fn upgrade_and_miss_post() {
    let store = &MemoryBlockstore::new();
    let policy = Policy::default();
    let (mut v, sector_info, worker, miner_id, deadline_index, partition_index, sector_size) =
        create_miner_and_upgrade_sector(store);
    let sector_number = sector_info.sector_number;

    let power_after_update = miner_power(&v, miner_id);
    assert!(!power_after_update.is_zero());

    // immediately miss post, lose power, become faulty
    v = advance_by_deadline_to_index(v, miner_id, deadline_index).0;
    v = advance_by_deadline_to_index(
        v,
        miner_id,
        deadline_index + 1 % policy.wpost_period_deadlines,
    )
    .0;
    assert!(!check_sector_active(&v, miner_id, sector_number));
    assert!(check_sector_faulty(&v, miner_id, deadline_index, partition_index, sector_number));

    let deadline_state = deadline_state(&v, miner_id, deadline_index);
    assert_eq!(power_after_update, deadline_state.faulty_power);

    let empty_sectors_array =
        Array::<(), MemoryBlockstore>::new_with_bit_width(store, SECTORS_AMT_BITWIDTH)
            .flush()
            .unwrap();
    assert_eq!(deadline_state.sectors_snapshot, empty_sectors_array);

    assert!(miner_power(&v, miner_id).is_zero());

    declare_recovery(&v, worker, miner_id, deadline_index, partition_index, sector_number);
    let (deadline_info, _, v) = advance_to_proving_deadline(v, miner_id, sector_number);

    submit_windowed_post(
        &v,
        worker,
        miner_id,
        deadline_info,
        partition_index,
        PowerPair {
            raw: StoragePower::from(sector_size as i64),
            qa: StoragePower::from(sector_size as i64),
        },
    );

    assert!(check_sector_active(&v, miner_id, sector_number));
    assert!(!check_sector_faulty(&v, miner_id, deadline_index, partition_index, sector_number));
    assert_eq!(miner_power(&v, miner_id).raw, BigInt::from(sector_size as i64));
    v.assert_state_invariants();
}

#[test]
fn prove_replica_update_multi_dline() {
    let store = &MemoryBlockstore::new();
    let policy = Policy::default();
    let mut v = VM::new_with_singletons(store);
    let addrs = create_accounts(&v, 1, TokenAmount::from(100_000e18 as i128));
    let (worker, owner) = (addrs[0], addrs[0]);
    let seal_proof = RegisteredSealProof::StackedDRG32GiBV1P1;
    let (maddr, _) = create_miner(
        &mut v,
        owner,
        worker,
        seal_proof.registered_window_post_proof().unwrap(),
        TokenAmount::from(10_000e18 as i128),
    );

    v = v.with_epoch(1440); // something offset far away from deadline 0 and 1

    /* Commit enough sectors to pack two partitions */
    let more_than_one_partition = 2440;
    let batch_size = 100;
    let first_sector_number_p1 = 0;
    let first_sector_number_p2 = seal_proof.window_post_partitions_sector().unwrap();
    let expiration = v.get_epoch() + policy.max_sector_expiration_extension;

    let new_precommits = precommit_sectors(
        &mut v,
        more_than_one_partition,
        batch_size,
        worker,
        maddr,
        seal_proof,
        first_sector_number_p1,
        true,
        Some(expiration),
    );

    let precommits = new_precommits;
    let to_prove = precommits;

    let prove_time = v.get_epoch() + policy.pre_commit_challenge_delay + 1;
    v = advance_by_deadline_to_epoch(v, maddr, prove_time).0;

    prove_commit_sectors(&mut v, worker, maddr, to_prove, batch_size);

    /* This is a mess, but it just ensures activation of both partitions by posting, cronning and checking */

    // advance to proving period and submit post for first partition
    let (deadline_info, partition_index, v) =
        advance_to_proving_deadline(v, maddr, first_sector_number_p1);

    // first partition shouldn't be active until PoSt
    assert!(!check_sector_active(&v, maddr, deadline_info.index));
    submit_windowed_post(&v, worker, maddr, deadline_info, partition_index, PowerPair::zero());

    // move into the next deadline so that the first batch of created sectors are active
    let (v, current_deadline_info) = advance_by_deadline_to_index(
        v,
        maddr,
        deadline_info.index + 1 % policy.wpost_period_deadlines,
    );

    // hooray, first partition is now active
    assert_eq!(1, current_deadline_info.index);
    assert!(check_sector_active(&v, maddr, first_sector_number_p1));
    assert!(check_sector_active(&v, maddr, first_sector_number_p1 + 1));
    assert!(check_sector_active(&v, maddr, first_sector_number_p1 + 2));
    assert!(check_sector_active(&v, maddr, first_sector_number_p1 + 2300));

    // second partition shouldn't be active until PoSt
    assert!(!check_sector_active(&v, maddr, first_sector_number_p2));
    submit_windowed_post(&v, worker, maddr, current_deadline_info, 0, PowerPair::zero());

    // move into the next deadline so that the second batch of created sectors are active
    let (v, _) = advance_by_deadline_to_index(
        v,
        maddr,
        deadline_info.index + 2 % policy.wpost_period_deadlines,
    );
    assert!(check_sector_active(&v, maddr, first_sector_number_p2));

    /* Replica Update across two deadlines */
    let old_sector_commr_p1 = sector_info(&v, maddr, first_sector_number_p1).sealed_cid;
    let old_sector_commr_p2 = sector_info(&v, maddr, first_sector_number_p2).sealed_cid;

    let deal_ids = create_deals(2, &v, worker, worker, maddr);

    let new_sealed_cid1 = make_sealed_cid(b"replica1");
    let replica_update_1 = ReplicaUpdate {
        sector_number: first_sector_number_p1,
        deadline: 0,
        partition: 0,
        new_sealed_cid: new_sealed_cid1,
        deals: deal_ids[0..1].to_vec(),
        update_proof_type: fvm_shared::sector::RegisteredUpdateProof::StackedDRG32GiBV1,
        replica_proof: vec![],
    };

    let new_sealed_cid2 = make_sealed_cid(b"replica2");
    let replica_update_2 = ReplicaUpdate {
        sector_number: first_sector_number_p2,
        deadline: 1,
        partition: 0,
        new_sealed_cid: new_sealed_cid2,
        deals: deal_ids[1..].to_vec(),
        update_proof_type: fvm_shared::sector::RegisteredUpdateProof::StackedDRG32GiBV1,
        replica_proof: vec![],
    };

    let ret_bf: BitField = apply_ok(
        &v,
        worker,
        maddr,
        TokenAmount::zero(),
        MethodsMiner::ProveReplicaUpdates as u64,
        ProveReplicaUpdatesParams { updates: vec![replica_update_1, replica_update_2] },
    )
    .deserialize()
    .unwrap();

    assert_eq!(ret_bf.len(), 2);
    assert!(ret_bf.get(first_sector_number_p1));
    assert!(ret_bf.get(first_sector_number_p2));

    let new_sector_info_p1 = sector_info(&v, maddr, first_sector_number_p1);
    assert_eq!(deal_ids[0], new_sector_info_p1.deal_ids[0]);
    assert_eq!(1, new_sector_info_p1.deal_ids.len());
    assert_eq!(old_sector_commr_p1, new_sector_info_p1.sector_key_cid.unwrap());
    assert_eq!(new_sealed_cid1, new_sector_info_p1.sealed_cid);
    let new_sector_info_p2 = sector_info(&v, maddr, first_sector_number_p2);
    assert_eq!(deal_ids[1], new_sector_info_p2.deal_ids[0]);
    assert_eq!(1, new_sector_info_p2.deal_ids.len());
    assert_eq!(old_sector_commr_p2, new_sector_info_p2.sector_key_cid.unwrap());
    assert_eq!(new_sealed_cid2, new_sector_info_p2.sealed_cid);
    v.assert_state_invariants();
}

// ---- Failure cases ----

// Tests that a sector in an immutable deadline cannot be upgraded
#[test]
fn immutable_deadline_failure() {
    let store = &MemoryBlockstore::new();
    let mut v = VM::new_with_singletons(store);
    let addrs = create_accounts(&v, 1, TokenAmount::from(100_000e18 as i128));
    let (worker, owner) = (addrs[0], addrs[0]);
    let seal_proof = RegisteredSealProof::StackedDRG32GiBV1P1;
    let (maddr, robust) = create_miner(
        &mut v,
        owner,
        worker,
        seal_proof.registered_window_post_proof().unwrap(),
        TokenAmount::from(10_000e18 as i128),
    );

    // advance to have seal randomness epoch in the past
    let v = v.with_epoch(200);

    let sector_number = 100;
    let (mut v, d_idx, p_idx) = create_sector(v, worker, maddr, sector_number, seal_proof);

    // make some deals
    let deal_ids = create_deals(1, &v, worker, worker, maddr);

    // Advance back into the sector's deadline
    v = advance_to_proving_deadline(v, maddr, sector_number).2;

    // replicaUpdate the sector
    let new_cid = make_sealed_cid(b"replica1");
    let replica_update = ReplicaUpdate {
        sector_number,
        deadline: d_idx,
        partition: p_idx,
        new_sealed_cid: new_cid,
        deals: deal_ids,
        update_proof_type: fvm_shared::sector::RegisteredUpdateProof::StackedDRG32GiBV1,
        replica_proof: vec![],
    };
    apply_code(
        &v,
        worker,
        robust,
        TokenAmount::zero(),
        MethodsMiner::ProveReplicaUpdates as u64,
        ProveReplicaUpdatesParams { updates: vec![replica_update] },
        ExitCode::USR_ILLEGAL_ARGUMENT,
    );
    v.assert_state_invariants();
}

#[test]
fn unhealthy_sector_failure() {
    let store = &MemoryBlockstore::new();
    let policy = Policy::default();
    let mut v = VM::new_with_singletons(store);
    let addrs = create_accounts(&v, 1, TokenAmount::from(100_000e18 as i128));
    let (worker, owner) = (addrs[0], addrs[0]);
    let seal_proof = RegisteredSealProof::StackedDRG32GiBV1P1;
    let (maddr, robust) = create_miner(
        &mut v,
        owner,
        worker,
        seal_proof.registered_window_post_proof().unwrap(),
        TokenAmount::from(10_000e18 as i128),
    );

    // advance to have seal randomness epoch in the past
    let v = v.with_epoch(200);

    let sector_number = 100;
    let (mut v, d_idx, p_idx) = create_sector(v, worker, maddr, sector_number, seal_proof);

    // make some deals
    let deal_ids = create_deals(1, &v, worker, worker, maddr);

    // ffw 2 days, missing posts
    let two_days_later = v.get_epoch() + policy.wpost_proving_period * 2;
    v = advance_by_deadline_to_epoch(v, maddr, two_days_later).0;
    assert!(!check_sector_active(&v, maddr, sector_number));
    assert!(check_sector_faulty(&v, maddr, d_idx, p_idx, sector_number));

    // replicaUpdate the sector
    let new_cid = make_sealed_cid(b"replica1");
    let replica_update = ReplicaUpdate {
        sector_number,
        deadline: d_idx,
        partition: p_idx,
        new_sealed_cid: new_cid,
        deals: deal_ids,
        update_proof_type: fvm_shared::sector::RegisteredUpdateProof::StackedDRG32GiBV1,
        replica_proof: vec![],
    };
    apply_code(
        &v,
        worker,
        robust,
        TokenAmount::zero(),
        MethodsMiner::ProveReplicaUpdates as u64,
        ProveReplicaUpdatesParams { updates: vec![replica_update] },
        ExitCode::USR_ILLEGAL_ARGUMENT,
    );
    v.expect_state_invariants(&[invariant_failure_patterns::REWARD_STATE_EPOCH_MISMATCH.to_owned()]);
}

#[test]
fn terminated_sector_failure() {
    let store = &MemoryBlockstore::new();
    let mut v = VM::new_with_singletons(store);
    let addrs = create_accounts(&v, 1, TokenAmount::from(100_000e18 as i128));
    let (worker, owner) = (addrs[0], addrs[0]);
    let seal_proof = RegisteredSealProof::StackedDRG32GiBV1P1;
    let (maddr, robust) = create_miner(
        &mut v,
        owner,
        worker,
        seal_proof.registered_window_post_proof().unwrap(),
        TokenAmount::from(10_000e18 as i128),
    );

    // advance to have seal randomness epoch in the past
    let v = v.with_epoch(200);

    let sector_number = 100;
    let (v, d_idx, p_idx) = create_sector(v, worker, maddr, sector_number, seal_proof);

    // make some deals
    let deal_ids = create_deals(1, &v, worker, worker, maddr);

    // terminate sector

    let terminate_parms = TerminateSectorsParams {
        terminations: vec![TerminationDeclaration {
            deadline: d_idx,
            partition: p_idx,
            sectors: make_bitfield(&[sector_number]),
        }],
    };
    apply_ok(
        &v,
        worker,
        maddr,
        TokenAmount::zero(),
        MethodsMiner::TerminateSectors as u64,
        terminate_parms,
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
        replica_proof: vec![],
    };
    apply_code(
        &v,
        worker,
        robust,
        TokenAmount::zero(),
        MethodsMiner::ProveReplicaUpdates as u64,
        ProveReplicaUpdatesParams { updates: vec![replica_update] },
        ExitCode::USR_ILLEGAL_ARGUMENT,
    );
    v.assert_state_invariants();
}

#[test]
fn bad_batch_size_failure() {
    let store = &MemoryBlockstore::new();
    let policy = Policy::default();
    let mut v = VM::new_with_singletons(store);
    let addrs = create_accounts(&v, 1, TokenAmount::from(100_000e18 as i128));
    let (worker, owner) = (addrs[0], addrs[0]);
    let seal_proof = RegisteredSealProof::StackedDRG32GiBV1P1;
    let (maddr, robust) = create_miner(
        &mut v,
        owner,
        worker,
        seal_proof.registered_window_post_proof().unwrap(),
        TokenAmount::from(10_000e18 as i128),
    );

    // advance to have seal randomness epoch in the past
    let v = v.with_epoch(200);

    let sector_number = 100;
    let (v, d_idx, p_idx) = create_sector(v, worker, maddr, sector_number, seal_proof);

    // make some deals
    let deal_ids = create_deals(1, &v, worker, worker, maddr);

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
            replica_proof: vec![],
        });
    }

    apply_code(
        &v,
        worker,
        robust,
        TokenAmount::zero(),
        MethodsMiner::ProveReplicaUpdates as u64,
        ProveReplicaUpdatesParams { updates },
        ExitCode::USR_ILLEGAL_ARGUMENT,
    );
    v.assert_state_invariants();
}

#[test]
fn no_dispute_after_upgrade() {
    let store = &MemoryBlockstore::new();
    let (v, _, worker, miner_id, deadline_index, _, _) = create_miner_and_upgrade_sector(store);

    let dispute_params = DisputeWindowedPoStParams { deadline: deadline_index, post_index: 0 };
    apply_code(
        &v,
        worker,
        miner_id,
        TokenAmount::zero(),
        MethodsMiner::DisputeWindowedPoSt as u64,
        dispute_params,
        ExitCode::USR_ILLEGAL_ARGUMENT,
    );
    v.assert_state_invariants();
}

#[test]
fn upgrade_bad_post_dispute() {
    let store = &MemoryBlockstore::new();
    let policy = Policy::default();
    let (v, sector_info, worker, miner_id, deadline_index, partition_index, _) =
        create_miner_and_upgrade_sector(store);
    let sector_number = sector_info.sector_number;

    let (deadline_info, _, mut v) = advance_to_proving_deadline(v, miner_id, sector_number);
    submit_invalid_post(&v, worker, miner_id, deadline_info, partition_index);

    v = advance_by_deadline_to_index(
        v,
        miner_id,
        deadline_index + 2 % policy.wpost_period_deadlines,
    )
    .0;

    let dispute_params = DisputeWindowedPoStParams { deadline: deadline_index, post_index: 0 };
    apply_ok(
        &v,
        worker,
        miner_id,
        TokenAmount::zero(),
        MethodsMiner::DisputeWindowedPoSt as u64,
        dispute_params,
    );
    v.assert_state_invariants();
}

#[test]
fn bad_post_upgrade_dispute() {
    let store = &MemoryBlockstore::new();
    let policy = Policy::default();
    let mut v = VM::new_with_singletons(store);
    let addrs = create_accounts(&v, 1, TokenAmount::from(100_000e18 as i128));
    let (worker, owner) = (addrs[0], addrs[0]);
    let seal_proof = RegisteredSealProof::StackedDRG32GiBV1P1;
    let (maddr, robust) = create_miner(
        &mut v,
        owner,
        worker,
        seal_proof.registered_window_post_proof().unwrap(),
        TokenAmount::from(10_000e18 as i128),
    );

    // advance to have seal randomness epoch in the past
    let v = v.with_epoch(200);

    let sector_number = 100;
    let (v, d_idx, p_idx) = create_sector(v, worker, maddr, sector_number, seal_proof);
    let old_sector_info = sector_info(&v, maddr, sector_number);

    // submit an invalid post
    let (deadline_info, _, mut v) = advance_to_proving_deadline(v, maddr, sector_number);

    submit_invalid_post(&v, worker, maddr, deadline_info, p_idx);
    v = advance_by_deadline_to_index(v, maddr, d_idx + 2 % policy.wpost_period_deadlines).0;

    // make some deals
    let deal_ids = create_deals(1, &v, worker, worker, maddr);

    // replicaUpdate the sector -- it succeeds
    let new_cid = make_sealed_cid(b"replica1");
    let replica_update = ReplicaUpdate {
        sector_number,
        deadline: d_idx,
        partition: p_idx,
        new_sealed_cid: new_cid,
        deals: deal_ids.clone(),
        update_proof_type: fvm_shared::sector::RegisteredUpdateProof::StackedDRG32GiBV1,
        replica_proof: vec![],
    };

    let updated_sectors: BitField = apply_ok(
        &v,
        worker,
        robust,
        TokenAmount::zero(),
        MethodsMiner::ProveReplicaUpdates as u64,
        ProveReplicaUpdatesParams { updates: vec![replica_update] },
    )
    .deserialize()
    .unwrap();
    assert_eq!(vec![100], bf_all(updated_sectors));

    // sanity check the sector after update
    let new_sector_info = sector_info(&v, maddr, sector_number);
    assert_eq!(1, new_sector_info.deal_ids.len());
    assert_eq!(deal_ids[0], new_sector_info.deal_ids[0]);
    assert_eq!(old_sector_info.sealed_cid, new_sector_info.sector_key_cid.unwrap());
    assert_eq!(new_cid, new_sector_info.sealed_cid);

    // BUT, I can still dispute your evil PoSt

    let dispute_params = DisputeWindowedPoStParams { deadline: d_idx, post_index: 0 };
    apply_ok(
        &v,
        worker,
        maddr,
        TokenAmount::zero(),
        MethodsMiner::DisputeWindowedPoSt as u64,
        dispute_params,
    );
    v.assert_state_invariants();
}

// Tests that an active CC sector can be correctly upgraded, and then the sector can be terminated
#[test]
fn terminate_after_upgrade() {
    let store = &MemoryBlockstore::new();
    let (v, sector_info, worker, miner_id, deadline_index, partition_index, _) =
        create_miner_and_upgrade_sector(store);
    let sector_number = sector_info.sector_number;

    let terminate_params = TerminateSectorsParams {
        terminations: vec![TerminationDeclaration {
            deadline: deadline_index,
            partition: partition_index,
            sectors: make_bitfield(&[sector_number]),
        }],
    };

    apply_ok(
        &v,
        worker,
        miner_id,
        TokenAmount::zero(),
        MethodsMiner::TerminateSectors as u64,
        terminate_params,
    );

    // expect power, market and miner to be in base state
    let miner_balances = v.get_miner_balance(miner_id);
    assert!(miner_balances.initial_pledge.is_zero());
    assert!(miner_balances.pre_commit_deposit.is_zero());

    let network_stats = v.get_network_stats();
    assert!(network_stats.miner_above_min_power_count.is_zero());
    assert!(network_stats.total_raw_byte_power.is_zero());
    assert!(network_stats.total_quality_adj_power.is_zero());
    assert!(network_stats.total_bytes_committed.is_zero());
    assert!(network_stats.total_qa_bytes_committed.is_zero());
    assert!(network_stats.total_pledge_collateral.is_zero());

    v.assert_state_invariants();
}

// Tests that an active CC sector can be correctly upgraded, and then the sector can be terminated
#[test]
fn extend_after_upgrade() {
    let store = &MemoryBlockstore::new();
    let policy = Policy::default();
    let (v, sector_info, worker, miner_id, deadline_index, partition_index, _) =
        create_miner_and_upgrade_sector(store);
    let sector_number = sector_info.sector_number;

    let extension_params = ExtendSectorExpirationParams {
        extensions: vec![ExpirationExtension {
            deadline: deadline_index,
            partition: partition_index,
            sectors: make_bitfield(&[sector_number]),
            new_expiration: v.get_epoch() + policy.max_sector_expiration_extension - 1,
        }],
    };

    apply_ok(
        &v,
        worker,
        miner_id,
        TokenAmount::zero(),
        MethodsMiner::ExtendSectorExpiration as u64,
        extension_params,
    );

    let miner_state = v.get_state::<MinerState>(miner_id).unwrap();
    let final_sector_info = miner_state.get_sector(store, sector_number).unwrap().unwrap();
    assert_eq!(
        policy.max_sector_expiration_extension - 1,
        final_sector_info.expiration - final_sector_info.activation
    );
    v.assert_state_invariants();
}

#[test]
fn wrong_deadline_index_failure() {
    let store = &MemoryBlockstore::new();
    let policy = Policy::default();
    let mut v = VM::new_with_singletons(store);
    let addrs = create_accounts(&v, 1, TokenAmount::from(100_000e18 as i128));
    let (worker, owner) = (addrs[0], addrs[0]);
    let seal_proof = RegisteredSealProof::StackedDRG32GiBV1P1;
    let (maddr, robust) = create_miner(
        &mut v,
        owner,
        worker,
        seal_proof.registered_window_post_proof().unwrap(),
        TokenAmount::from(10_000e18 as i128),
    );

    // advance to have seal randomness epoch in the past
    let v = v.with_epoch(200);

    let sector_number = 100;
    let (v, d_idx, p_idx) = create_sector(v, worker, maddr, sector_number, seal_proof);
    let old_sector_info = sector_info(&v, maddr, sector_number);

    // make some deals
    let deal_ids = create_deals(1, &v, worker, worker, maddr);

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
            replica_proof: vec![],
        });
    }

    apply_code(
        &v,
        worker,
        robust,
        TokenAmount::zero(),
        MethodsMiner::ProveReplicaUpdates as u64,
        ProveReplicaUpdatesParams { updates },
        ExitCode::USR_ILLEGAL_ARGUMENT,
    );

    let new_sector_info = sector_info(&v, maddr, sector_number);
    assert_eq!(old_sector_info, new_sector_info);
    v.assert_state_invariants();
}

#[test]
fn wrong_partition_index_failure() {
    let store = &MemoryBlockstore::new();
    let policy = Policy::default();
    let mut v = VM::new_with_singletons(store);
    let addrs = create_accounts(&v, 1, TokenAmount::from(100_000e18 as i128));
    let (worker, owner) = (addrs[0], addrs[0]);
    let seal_proof = RegisteredSealProof::StackedDRG32GiBV1P1;
    let (maddr, robust) = create_miner(
        &mut v,
        owner,
        worker,
        seal_proof.registered_window_post_proof().unwrap(),
        TokenAmount::from(10_000e18 as i128),
    );

    // advance to have seal randomness epoch in the past
    let v = v.with_epoch(200);

    let sector_number = 100;
    let (v, d_idx, p_idx) = create_sector(v, worker, maddr, sector_number, seal_proof);
    let old_sector_info = sector_info(&v, maddr, sector_number);

    // make some deals
    let deal_ids = create_deals(1, &v, worker, worker, maddr);

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
            replica_proof: vec![],
        });
    }

    apply_code(
        &v,
        worker,
        robust,
        TokenAmount::zero(),
        MethodsMiner::ProveReplicaUpdates as u64,
        ProveReplicaUpdatesParams { updates },
        ExitCode::USR_ILLEGAL_ARGUMENT,
    );

    let new_sector_info = sector_info(&v, maddr, sector_number);
    assert_eq!(old_sector_info, new_sector_info);
    v.assert_state_invariants();
}

#[test]
fn deal_included_in_multiple_sectors_failure() {
    let store = &MemoryBlockstore::new();
    let policy = Policy::default();
    let mut v = VM::new_with_singletons(store);
    let addrs = create_accounts(&v, 1, TokenAmount::from(100_000e18 as i128));
    let (worker, owner) = (addrs[0], addrs[0]);
    let seal_proof = RegisteredSealProof::StackedDRG32GiBV1P1;
    let (maddr, _) = create_miner(
        &mut v,
        owner,
        worker,
        seal_proof.registered_window_post_proof().unwrap(),
        TokenAmount::from(10_000e18 as i128),
    );

    //
    // preCommit two sectors
    //
    //

    let first_sector_number = 100;
    let precommits = precommit_sectors(
        &mut v,
        policy.min_aggregated_sectors,
        policy.pre_commit_sector_batch_max_size as i64,
        worker,
        maddr,
        seal_proof,
        first_sector_number,
        true,
        None,
    );

    assert_eq!(policy.min_aggregated_sectors, precommits.len() as u64);

    let miner_balance = v.get_miner_balance(maddr);
    assert!(miner_balance.pre_commit_deposit.is_positive());

    let prove_time = v.get_epoch() + policy.pre_commit_challenge_delay + 1;
    v = advance_by_deadline_to_epoch(v, maddr, prove_time).0;

    prove_commit_sectors(&mut v, worker, maddr, precommits, 100);

    // In the same epoch, trigger cron to validate prove commit
    apply_ok(
        &v,
        *SYSTEM_ACTOR_ADDR,
        *CRON_ACTOR_ADDR,
        TokenAmount::zero(),
        MethodsCron::EpochTick as u64,
        RawBytes::default(),
    );

    // advance to proving period and submit post
    let (deadline_info, partition_index, mut v) =
        advance_to_proving_deadline(v, maddr, first_sector_number);

    // sector shouldn't be active until PoSt
    assert!(!check_sector_active(&v, maddr, first_sector_number));
    assert!(!check_sector_active(&v, maddr, first_sector_number + 1));

    submit_windowed_post(&v, worker, maddr, deadline_info, partition_index, PowerPair::zero());

    // move into the next deadline so that the created sectors are mutable
    v = advance_by_deadline_to_index(
        v,
        maddr,
        deadline_info.index + 1 % policy.wpost_period_deadlines,
    )
    .0;

    // sectors are now active!
    assert!(check_sector_active(&v, maddr, first_sector_number));
    assert!(check_sector_active(&v, maddr, first_sector_number + 1));

    // make some unverified deals

    let deal_ids = create_deals(2, &v, worker, worker, maddr);

    // replicaUpdate the sector

    let new_sealed_cid1 = make_sealed_cid(b"replica1");
    let replica_update_1 = ReplicaUpdate {
        sector_number: first_sector_number,
        deadline: 0,
        partition: 0,
        new_sealed_cid: new_sealed_cid1,
        deals: deal_ids.clone(),
        update_proof_type: fvm_shared::sector::RegisteredUpdateProof::StackedDRG32GiBV1,
        replica_proof: vec![],
    };

    let new_sealed_cid2 = make_sealed_cid(b"replica2");
    let replica_update_2 = ReplicaUpdate {
        sector_number: first_sector_number + 1,
        deadline: 0,
        partition: 0,
        new_sealed_cid: new_sealed_cid2,
        deals: deal_ids.clone(),
        update_proof_type: fvm_shared::sector::RegisteredUpdateProof::StackedDRG32GiBV1,
        replica_proof: vec![],
    };

    let ret_bf: BitField = apply_ok(
        &v,
        worker,
        maddr,
        TokenAmount::zero(),
        MethodsMiner::ProveReplicaUpdates as u64,
        ProveReplicaUpdatesParams { updates: vec![replica_update_1, replica_update_2] },
    )
    .deserialize()
    .unwrap();

    assert_eq!(ret_bf.len(), 1);
    assert!(ret_bf.get(first_sector_number));
    assert!(!ret_bf.get(first_sector_number + 1));

    let new_sector_info_p1 = sector_info(&v, maddr, first_sector_number);
    assert_eq!(deal_ids, new_sector_info_p1.deal_ids);
    assert_eq!(new_sealed_cid1, new_sector_info_p1.sealed_cid);

    let new_sector_info_p2 = sector_info(&v, maddr, first_sector_number + 1);
    assert!(new_sector_info_p2.deal_ids.len().is_zero());
    assert_ne!(new_sealed_cid2, new_sector_info_p2.sealed_cid);
    v.assert_state_invariants();
}

fn create_miner_and_upgrade_sector(
    store: &MemoryBlockstore,
) -> (VM, SectorOnChainInfo, Address, Address, u64, u64, SectorSize) {
    let mut v = VM::new_with_singletons(store);
    let addrs = create_accounts(&v, 1, TokenAmount::from(100_000e18 as i128));
    let (worker, owner) = (addrs[0], addrs[0]);
    let seal_proof = RegisteredSealProof::StackedDRG32GiBV1P1;
    let (maddr, robust) = create_miner(
        &mut v,
        owner,
        worker,
        seal_proof.registered_window_post_proof().unwrap(),
        TokenAmount::from(10_000e18 as i128),
    );

    // advance to have seal randomness epoch in the past
    let v = v.with_epoch(200);

    let sector_number = 100;
    let (v, d_idx, p_idx) = create_sector(v, worker, maddr, sector_number, seal_proof);

    let old_sector_info = sector_info(&v, maddr, sector_number);
    // make some deals
    let deal_ids = create_deals(1, &v, worker, worker, maddr);

    // replica update
    let new_cid = make_sealed_cid(b"replica1");
    let replica_update = ReplicaUpdate {
        sector_number,
        deadline: d_idx,
        partition: p_idx,
        new_sealed_cid: new_cid,
        deals: deal_ids.clone(),
        update_proof_type: fvm_shared::sector::RegisteredUpdateProof::StackedDRG32GiBV1,
        replica_proof: vec![],
    };
    let updated_sectors: BitField = apply_ok(
        &v,
        worker,
        robust,
        TokenAmount::zero(),
        MethodsMiner::ProveReplicaUpdates as u64,
        ProveReplicaUpdatesParams { updates: vec![replica_update] },
    )
    .deserialize()
    .unwrap();
    assert_eq!(vec![100], bf_all(updated_sectors));

    // sanity check the sector after update
    let new_sector_info = sector_info(&v, maddr, sector_number);
    assert_eq!(1, new_sector_info.deal_ids.len());
    assert_eq!(deal_ids[0], new_sector_info.deal_ids[0]);
    assert_eq!(old_sector_info.sealed_cid, new_sector_info.sector_key_cid.unwrap());
    assert_eq!(new_cid, new_sector_info.sealed_cid);
    (v, new_sector_info, worker, maddr, d_idx, p_idx, seal_proof.sector_size().unwrap())
}

// This method produces an active, mutable sector, by:
// - PreCommiting a sector
// - fastforwarding time and ProveCommitting it
// - fastforwarding to its Proving period and PoSting it
// - fastforwarding out of the proving period into a new deadline
// This method assumes that this is a miners first and only sector
fn create_sector(
    mut v: VM,
    worker: Address,
    maddr: Address,
    sector_number: SectorNumber,
    seal_proof: RegisteredSealProof,
) -> (VM, u64, u64) {
    // precommit
    let exp = v.get_epoch() + Policy::default().max_sector_expiration_extension;
    let precommits =
        precommit_sectors(&mut v, 1, 1, worker, maddr, seal_proof, sector_number, true, Some(exp));
    assert_eq!(1, precommits.len());
    assert_eq!(sector_number, precommits[0].info.sector_number);
    let balances = v.get_miner_balance(maddr);
    assert!(balances.pre_commit_deposit.is_positive());

    // prove commit
    let prove_time = v.get_epoch() + Policy::default().pre_commit_challenge_delay + 1;
    let v = advance_by_deadline_to_epoch(v, maddr, prove_time).0;
    let prove_commit_params = ProveCommitSectorParams { sector_number, proof: vec![] };
    apply_ok(
        &v,
        worker,
        maddr,
        TokenAmount::zero(),
        MethodsMiner::ProveCommitSector as u64,
        prove_commit_params,
    );
    let res = v
        .apply_message(
            *SYSTEM_ACTOR_ADDR,
            *CRON_ACTOR_ADDR,
            TokenAmount::zero(),
            MethodsCron::EpochTick as u64,
            RawBytes::default(),
        )
        .unwrap();
    assert_eq!(ExitCode::OK, res.code);
    let (dline_info, p_idx, v) = advance_to_proving_deadline(v, maddr, sector_number);
    let d_idx = dline_info.index;
    // not active until post
    assert!(!check_sector_active(&v, maddr, sector_number));
    let m_st = v.get_state::<MinerState>(maddr).unwrap();
    let sector = m_st.get_sector(v.store, sector_number).unwrap().unwrap();
    let sector_power = power_for_sector(seal_proof.sector_size().unwrap(), &sector);
    submit_windowed_post(&v, worker, maddr, dline_info, p_idx, sector_power);

    // move to next deadline to activate power
    let v = advance_by_deadline_to_index(
        v,
        maddr,
        d_idx + 1 % Policy::default().wpost_period_deadlines,
    )
    .0;

    // hooray sector is now active
    assert!(check_sector_active(&v, maddr, sector_number));

    // sanity check the sector
    let old_sector_info = sector_info(&v, maddr, sector_number);
    assert!(old_sector_info.deal_ids.is_empty());
    assert_eq!(None, old_sector_info.sector_key_cid);
    let miner_power = miner_power(&v, maddr);
    assert_eq!(StoragePower::from(seal_proof.sector_size().unwrap() as u64), miner_power.raw);

    (v, d_idx, p_idx)
}

fn create_deals(
    num_deals: u128,
    v: &VM,
    client: Address,
    worker: Address,
    maddr: Address,
) -> Vec<DealID> {
    let collateral = TokenAmount::from(3 * num_deals * 1e18 as u128);
    apply_ok(
        v,
        client,
        *STORAGE_MARKET_ACTOR_ADDR,
        collateral.clone(),
        MethodsMarket::AddBalance as u64,
        client,
    );
    apply_ok(
        v,
        worker,
        *STORAGE_MARKET_ACTOR_ADDR,
        collateral,
        MethodsMarket::AddBalance as u64,
        maddr,
    );

    let mut ids = Vec::<DealID>::new();
    let deal_start = v.get_epoch() + Policy::default().pre_commit_challenge_delay + 1;

    for i in 0..num_deals {
        let deals = publish_deal(
            v,
            worker,
            client,
            maddr,
            format!("deal-label {}", i),
            PaddedPieceSize(32 << 30),
            false,
            deal_start,
            180 * EPOCHS_IN_DAY,
        );
        for id in deals.ids.iter() {
            ids.push(*id);
        }
    }
    ids
}
