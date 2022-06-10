
use fil_actor_market::{
     Method as MethodsMarket,
};use fil_actor_miner::{
    power_for_sector, Method as MethodsMiner, ProveCommitSectorParams,
    SectorOnChainInfo, State as MinerState, ReplicaUpdate, ProveReplicaUpdatesParams,
};
use fil_actor_cron::Method as MethodsCron;


use fil_actors_runtime::test_utils::make_sealed_cid;
use fvm_shared::piece::PaddedPieceSize;

use std::fmt::Debug;
use fil_actors_runtime::runtime::Policy;
use fil_actors_runtime::{
    CRON_ACTOR_ADDR, STORAGE_MARKET_ACTOR_ADDR,
    SYSTEM_ACTOR_ADDR, EPOCHS_IN_DAY,
};
use fvm_ipld_bitfield::{BitField};
use fvm_shared::deal::DealID;
use fvm_ipld_blockstore::MemoryBlockstore;
use fvm_ipld_encoding::RawBytes;
use fvm_shared::address::Address;
use fvm_shared::bigint::Zero;
use fvm_shared::sector::SectorSize;
use fvm_shared::econ::TokenAmount;
use fvm_shared::error::ExitCode;
use fvm_shared::sector::{RegisteredSealProof, SectorNumber};
use num_traits::sign::Signed;
use fvm_shared::sector::StoragePower;
use test_vm::util::{
    advance_by_deadline_to_epoch, bf_all, advance_to_proving_deadline, apply_ok, create_accounts, publish_deal,
    create_miner, precommit_sectors, submit_windowed_post, check_sector_active, advance_by_deadline_to_index, sector_info, miner_power
};
use test_vm::{VM};

#[test]
fn replica_update_simple_path_sucess() {
    create_miner_and_upgrade_sector(&MemoryBlockstore::new());
}

fn create_miner_and_upgrade_sector<'bs>(
    store: &'bs MemoryBlockstore,
) -> (VM<'bs>, SectorOnChainInfo, Address, Address, u64, u64, SectorSize) {

    let mut v = VM::new_with_singletons(&store);
    let addrs = create_accounts(&v, 1, TokenAmount::from(100_000e18 as i128));
    let (worker, owner) = (addrs[0], addrs[0]);
    let seal_proof = RegisteredSealProof::StackedDRG32GiBV1P1;
    let (maddr, robust) = create_miner(
        & mut v,
        owner,
        worker,
        seal_proof.registered_window_post_proof().unwrap(),
        TokenAmount::from(10_000e18 as i128),
    );

    // advance to have seal randomness epoch in the past
    let v = v.with_epoch(200);

    let sector_number = 100;
    let (v, d_idx, p_idx) = create_sector(v, worker, maddr, sector_number, seal_proof);

    // sanity check the sector
    let old_sector_info = sector_info(&v, maddr, sector_number);
    assert_eq!(Vec::<u64>::new(), old_sector_info.deal_ids);
    assert_eq!(None, old_sector_info.sector_key_cid);
    let miner_power = miner_power(&v, maddr);
    assert_eq!(StoragePower::from(seal_proof.sector_size().unwrap() as u64), miner_power.raw);

    // make some deals
    let deal_ids = create_deals(1, &v, worker, worker, maddr, seal_proof);
    
    // replica update
    let new_cid = make_sealed_cid(b"replica1");
    let replica_update = ReplicaUpdate{
        sector_number,
        deadline: d_idx,
        partition: p_idx,
        new_sealed_cid: new_cid,
        deals: deal_ids.clone(),
        update_proof_type: fvm_shared::sector::RegisteredUpdateProof::StackedDRG32GiBV1,
        replica_proof: vec![],
    };
    let updated_sectors: BitField = apply_ok(&v, worker, robust, TokenAmount::zero(), MethodsMiner::ProveReplicaUpdates as u64, ProveReplicaUpdatesParams{
        updates: vec![replica_update],
    }).deserialize().unwrap();
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
    let exp  = v.get_epoch() + Policy::default().max_sector_expiration_extension;
    let precommits = precommit_sectors(
        &mut v,
        1,
        1,
        worker,
        maddr,
        seal_proof,
        sector_number,
        true,
        Some(exp),
    );
    assert_eq!(1, precommits.len());
    assert_eq!(sector_number, precommits[0].info.sector_number);
    let balances = v.get_miner_balance(maddr);
    assert!(balances.pre_commit_deposit.is_positive());

    // prove commit
    let prove_time = v.get_epoch() + Policy::default().pre_commit_challenge_delay + 1;
    let v = advance_by_deadline_to_epoch(v, maddr, prove_time).0;
    let prove_commit_params = ProveCommitSectorParams{
        sector_number,
        proof: vec![],
    };
    apply_ok(&v, worker, maddr, TokenAmount::zero(), MethodsMiner::ProveCommitSector as u64, prove_commit_params);
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
    let v = advance_by_deadline_to_index(v,  maddr, d_idx + 1 % Policy::default().wpost_period_deadlines).0;

    // hooray sector is now active
    assert!(check_sector_active(&v, maddr, sector_number));
    
    (v, d_idx, p_idx)
}

fn create_deals(num_deals: u128, v: &VM, client: Address, worker: Address, maddr: Address, seal_proof: RegisteredSealProof) -> Vec<DealID> {
    let collateral = TokenAmount::from(3*num_deals * 1e18 as u128);
    apply_ok(&v, client, *STORAGE_MARKET_ACTOR_ADDR, collateral.clone(), MethodsMarket::AddBalance as u64, client);
    apply_ok(&v, worker, *STORAGE_MARKET_ACTOR_ADDR, collateral, MethodsMarket::AddBalance as u64, maddr);

    let mut ids = Vec::<DealID>::new();
    let deal_start = v.get_epoch() +  Policy::default().pre_commit_challenge_delay + 1;

    for i in 0..num_deals {
        let deals = publish_deal(&v, worker, client, maddr, "deal-label".to_string(), PaddedPieceSize(32<<30), false, deal_start, 180*EPOCHS_IN_DAY);
        for id in deals.ids.iter() {
            ids.push(*id);
        }
    }
    ids
}