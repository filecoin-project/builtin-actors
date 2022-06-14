use fil_actor_market::Method as MarketMethod;
use fil_actor_miner::{
    power_for_sector, Method as MinerMethod, ProveCommitAggregateParams,
    State as MinerState,
};
use fil_actor_miner::{PoStPartition, SectorPreCommitOnChainInfo};
use fil_actor_power::Method as PowerMethod;
use fil_actor_reward::Method as RewardMethod;
use fil_actor_state::check::Tree;
use fil_actors_runtime::cbor::serialize;
use fil_actors_runtime::builtin::SYSTEM_ACTOR_ADDR;
use fil_actors_runtime::runtime::policy::policy_constants::PRE_COMMIT_CHALLENGE_DELAY;
use fil_actors_runtime::runtime::policy_constants::{PRE_COMMIT_SECTOR_BATCH_MAX_SIZE, MAX_AGGREGATED_SECTORS};
use fil_actors_runtime::test_utils::*;
use fil_actors_runtime::{
    BURNT_FUNDS_ACTOR_ADDR, REWARD_ACTOR_ADDR, STORAGE_MARKET_ACTOR_ADDR,
    STORAGE_POWER_ACTOR_ADDR, CRON_ACTOR_ADDR,
};
use fil_actor_cron::Method as CronMethod;
use fvm_ipld_bitfield::BitField;
use fvm_ipld_bitfield::UnvalidatedBitField;
use fvm_ipld_blockstore::MemoryBlockstore;
use fvm_ipld_encoding::RawBytes;
use fvm_shared::address::Address;
use fvm_shared::bigint::{Zero, BigInt};
use fvm_shared::econ::TokenAmount;
use fvm_shared::sector::{RegisteredSealProof, SectorNumber};
use fvm_shared::METHOD_SEND;
use num_traits::Signed;
use test_vm::util::{
    advance_to_proving_deadline, apply_ok, create_accounts,
    create_miner, precommit_sectors, submit_windowed_post, make_bitfield,
};
use test_vm::{ExpectInvocation, VM};
use std::cmp::min;

struct Onboarding {
    epoch_delay: i64,                 // epochs to advance since the prior action
    pre_commit_sector_count: u64,     // sectors to batch pre-commit
    pre_commit_batch_size: i64,       // batch size (multiple batches if committing more)
    prove_commit_sector_count: u64,   // sectors to aggregate prove-commit
    prove_commit_aggregate_size: i64, // aggregate size (multiple aggregates if proving more)
}

impl Onboarding {
    fn new(epoch_delay: i64, pre_commit_sector_count: u64, pre_commit_batch_size: i64, prove_commit_sector_count: u64, prove_commit_aggregate_size: i64) -> Self {
        Self {
            epoch_delay,
            pre_commit_sector_count,
            pre_commit_batch_size,
            prove_commit_sector_count,
            prove_commit_aggregate_size,
        }
    }
}

#[test]
fn batch_onboarding() {
    let store = MemoryBlockstore::new();
    let mut v = VM::new_with_singletons(&store);
    let addrs = create_accounts(&v, 1, TokenAmount::from(10_000e18 as i128));
    let seal_proof = RegisteredSealProof::StackedDRG32GiBV1P1;
    let (owner, worker) = (addrs[0], addrs[0]);
    let (id_addr, _) = create_miner(
        &mut v,
        owner,
        worker,
        seal_proof.registered_window_post_proof().unwrap(),
        TokenAmount::from(10_000e18 as i128),
    );
    let mut v = v.with_epoch(200);

    // A series of pre-commit and prove-commit actions intended to cover paths including:
    // - different pre-commit batch sizes
    // - different prove-commit aggregate sizes
    // - multiple pre-commit batches before proof
    // - proving only some of the pre-commits
    // - proving part of multiple pre-commit batches
    // - proving all pre-commits across multiple batches
    // - interleaving of pre- and prove-commit
    //
    // Sectors are still proven in the order of pre-commitment.

    let mut next_sector_no: SectorNumber = 0;
    let mut pre_committed_count = 0;
    let mut proven_count = 0;

    let vec_onboarding = vec![
        Onboarding::new(0, 10, PRE_COMMIT_SECTOR_BATCH_MAX_SIZE as i64, 0, 0),
        Onboarding::new(1, 20, 12, 0, 0),
        Onboarding::new(PRE_COMMIT_CHALLENGE_DELAY + 1, 0, 0, 8, MAX_AGGREGATED_SECTORS as i64),
        Onboarding::new(1, 0, 0, 8, 4),
        Onboarding::new(1, 10, 4, 0, 0),
        Onboarding::new(PRE_COMMIT_CHALLENGE_DELAY + 1, 0, 0, 24, 10),
    ];

    let mut precommmits: Vec<SectorPreCommitOnChainInfo> = vec![];

    for item in vec_onboarding {
        let epoch = v.get_epoch();
        v = v.with_epoch(epoch + item.epoch_delay);

        if item.pre_commit_sector_count > 0 {
            let mut new_precommits = precommit_sectors(
                &mut v,
                item.pre_commit_sector_count,
                item.pre_commit_batch_size,
                worker,
                id_addr,
                seal_proof,
                next_sector_no,
                next_sector_no == 0,
                None,
            );
            precommmits.append(&mut new_precommits);
            next_sector_no += item.pre_commit_sector_count;
            pre_committed_count += item.pre_commit_sector_count;
        }

        if item.prove_commit_sector_count > 0 {
            let to_prove = precommmits[..item.prove_commit_sector_count as usize].to_vec();
            precommmits = precommmits[item.prove_commit_sector_count as usize..].to_vec();
            prove_commit_sectors(
                &mut v,
                worker,
                id_addr,
                to_prove,
                item.prove_commit_aggregate_size,
            );
            proven_count += item.prove_commit_sector_count;
        }
    }

    let (dline_info, p_idx, v) = advance_to_proving_deadline(v, id_addr, 0);

    // submit post
    let st = v.get_state::<MinerState>(id_addr).unwrap();
    let sector = st.get_sector(v.store, 0).unwrap().unwrap();

    let mut partitions = vec![PoStPartition { index: p_idx, skipped: UnvalidatedBitField::Validated(BitField::new()) }];
    let mut new_power = power_for_sector(seal_proof.sector_size().unwrap(), &sector);
    new_power.raw *= proven_count;
    new_power.qa *= proven_count;

    submit_windowed_post(&v, worker, id_addr, dline_info, p_idx, new_power.clone());

    let balances = v.get_miner_balance(id_addr);
    assert!(balances.initial_pledge.is_positive());

    let network_stats = v.get_network_stats();
    let sector_size = seal_proof.sector_size().unwrap() as u64;
    assert_eq!(network_stats.total_bytes_committed, BigInt::from(sector_size * proven_count));
    assert!(network_stats.total_pledge_collateral.is_positive());

    apply_ok(
        &v,
        *SYSTEM_ACTOR_ADDR,
        *CRON_ACTOR_ADDR,
        TokenAmount::zero(),
        CronMethod::EpochTick as u64,
        RawBytes::default(),
    );

    let state_tree = Tree::load(&store, &v.checkpoint()).unwrap();

    v.get_total_actor_balance(&store).unwrap();
    
    // v.check_state_invariants().unwrap().assert_empty()
}

pub fn prove_commit_sectors(
    v: &mut VM,
    worker: Address,
    maddr: Address,
    precommits: Vec<SectorPreCommitOnChainInfo>,
    aggregate_size: i64,
) {
    let mut precommit_infos = precommits.as_slice();
    while !precommit_infos.is_empty() {
        let batch_size = min(aggregate_size, precommit_infos.len() as i64) as usize;
        let to_prove = &precommit_infos[0..batch_size];
        precommit_infos = &precommit_infos[batch_size..];
        let sector_nos_bf =  make_bitfield(to_prove.iter().map(|p| p.info.sector_number).collect::<Vec<u64>>().as_slice());

        let prove_commit_aggregate_params = ProveCommitAggregateParams {
            sector_numbers: sector_nos_bf,
            aggregate_proof: vec![],
        };
        let prove_commit_aggregate_params_ser =
            serialize(&prove_commit_aggregate_params, "prove commit aggregate params").unwrap();

        apply_ok(
            v,
            worker,
            maddr,
            TokenAmount::zero(),
            MinerMethod::ProveCommitAggregate as u64,
            prove_commit_aggregate_params,
        );

        ExpectInvocation {
            to: maddr,
            method: MinerMethod::ProveCommitAggregate as u64,
            from: Some(worker),
            params: Some(prove_commit_aggregate_params_ser),
            subinvocs: Some(vec![
                ExpectInvocation {
                    to: *STORAGE_MARKET_ACTOR_ADDR,
                    method: MarketMethod::ComputeDataCommitment as u64,
                    ..Default::default()
                },
                ExpectInvocation {
                    to: *REWARD_ACTOR_ADDR,
                    method: RewardMethod::ThisEpochReward as u64,
                    ..Default::default()
                },
                ExpectInvocation {
                    to: *STORAGE_POWER_ACTOR_ADDR,
                    method: PowerMethod::CurrentTotalPower as u64,
                    ..Default::default()
                },
                ExpectInvocation {
                    to: *STORAGE_POWER_ACTOR_ADDR,
                    method: PowerMethod::UpdatePledgeTotal as u64,
                    ..Default::default()
                },
                ExpectInvocation {
                    to: *BURNT_FUNDS_ACTOR_ADDR,
                    method: METHOD_SEND,
                    ..Default::default()
                },
            ]),
            ..Default::default()
        }
        .matches(v.take_invocations().last().unwrap());
    }
}
