use fil_actor_cron::Method as CronMethod;
use fil_actor_market::Method as MarketMethod;
use fil_actor_miner::{
    max_prove_commit_duration, power_for_sector, Method as MethodsMiner, ProveCommitSectorParams,
    SectorOnChainInfo, State as MinerState,
};
use fil_actor_cron::Method as MethodsCron;
use fil_actor_power::{CreateMinerReturn, Method as PowerMethod, State as PowerState};
use fil_actor_reward::Method as RewardMethod;
use fil_actors_runtime::cbor::serialize;
use fil_actors_runtime::runtime::Policy;
use fil_actors_runtime::{
    CRON_ACTOR_ADDR, REWARD_ACTOR_ADDR, STORAGE_MARKET_ACTOR_ADDR, STORAGE_POWER_ACTOR_ADDR,
    SYSTEM_ACTOR_ADDR,
};
use fvm_ipld_blockstore::MemoryBlockstore;
use fvm_ipld_encoding::RawBytes;
use fvm_shared::address::Address;
use fvm_shared::bigint::Zero;
use fvm_shared::econ::TokenAmount;
use fvm_shared::error::ExitCode;
use fvm_shared::sector::{RegisteredSealProof, SectorNumber};
use num_traits::sign::Signed;
use test_vm::util::{
    advance_by_deadline_to_epoch, advance_to_proving_deadline, apply_ok, create_accounts,
    create_miner, precommit_sectors, submit_windowed_post,
};
use test_vm::{ExpectInvocation, VM};

#[test]
fn replica_update_simple_path_sucess() {
    create_miner_and_upgrade_sector(MemoryBlockstore::new());
}

fn create_miner_and_upgrade_sector<'bs>(
    store: MemoryBlockstore,
) -> (VM<'bs>, SectorOnChainInfo, Address, CreateMinerReturn, u64, u64, u64) {
    let mut v = VM::new_with_singletons(&store);
    let addrs = create_accounts(&v, 1, TokenAmount::from(10_000e18 as i128));
    let (worker, owner) = (addrs[0], addrs[0]);
    let seal_proof = RegisteredSealProof::StackedDRG32GiBV1P1;
    let maddr = create_miner(
        &v,
        owner,
        worker,
        seal_proof.registered_window_post_proof().unwrap(),
        TokenAmount::from(10_000e18 as i128),
    )
    .0;

    // advance to have seal randomness epoch in the past
    let v = v.with_epoch(200);

    let (v, d_idx, p_idx) = create_sector(v, worker, maddr, 100, seal_proof);

    (v, None, None, None, None, None)
}

// This method produces an active, mutable sector, by:
// - PreCommiting a sector
// - fastforwarding time and ProveCommitting it
// - fastforwarding to its Proving period and PoSting it
// - fastforwarding out of the proving period into a new deadline
// This method assumes that this is a miners first and only sector
fn create_sector(
    v: VM,
    worker: Address,
    maddr: Address,
    sector_number: SectorNumber,
    seal_proof: RegisteredSealProof,
) -> (VM, u64, u64) {
    // precommit
    let precommits = precommit_sectors(
        &mut v,
        1,
        1,
        worker,
        maddr,
        seal_proof,
        sector_number,
        true,
        Some(v.get_epoch() + Policy::default().max_sector_expiration_extension),
    );
    assert_eq!(1, precommits.len());
    assert_eq!(sector_number, precommits[0].info.sector_number);
    let balances = v.get_miner_balance(maddr);
    assert!(balances.pre_commit_deposit.is_positive());

    // prove commit
    let prove_time = v.get_epoch() + max_prove_commit_duration(&Policy::default(), seal_proof).unwrap();
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
    let (d_idx, p_dix, v) = advance_to_proving_deadline(v, maddr, sector_number);
    

    (v, 1, 1)
}
