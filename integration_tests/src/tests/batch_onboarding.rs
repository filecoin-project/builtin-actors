use export_macro::vm_test;
use fil_actor_miner::SectorPreCommitOnChainInfo;
use fil_actor_miner::{power_for_sector, State as MinerState};
use fil_actors_runtime::runtime::policy::policy_constants::PRE_COMMIT_CHALLENGE_DELAY;
use fil_actors_runtime::runtime::policy_constants::MAX_AGGREGATED_SECTORS;
use fil_actors_runtime::runtime::Policy;
use fvm_shared::bigint::BigInt;
use fvm_shared::econ::TokenAmount;
use fvm_shared::sector::{RegisteredSealProof, SectorNumber};
use vm_api::util::{get_state, DynBlockstore};
use vm_api::VM;

use crate::util::{
    advance_to_proving_deadline, create_accounts, create_miner, cron_tick, expect_invariants,
    get_network_stats, invariant_failure_patterns, miner_balance, precommit_sectors_v2,
    prove_commit_sectors, submit_windowed_post,
};

struct Onboarding {
    epoch_delay: i64,                   // epochs to advance since the prior action
    pre_commit_sector_count: usize,     // sectors to batch pre-commit
    pre_commit_batch_size: usize,       // batch size (multiple batches if committing more)
    prove_commit_sector_count: usize,   // sectors to aggregate prove-commit
    prove_commit_aggregate_size: usize, // aggregate size (multiple aggregates if proving more)
}

impl Onboarding {
    fn new(
        epoch_delay: i64,
        pre_commit_sector_count: usize,
        pre_commit_batch_size: usize,
        prove_commit_sector_count: usize,
        prove_commit_aggregate_size: usize,
    ) -> Self {
        Self {
            epoch_delay,
            pre_commit_sector_count,
            pre_commit_batch_size,
            prove_commit_sector_count,
            prove_commit_aggregate_size,
        }
    }
}

#[vm_test]
pub fn batch_onboarding_test(v: &dyn VM) {
    let seal_proof = &RegisteredSealProof::StackedDRG32GiBV1P1;

    let mut proven_count = 0;
    let addrs = create_accounts(v, 1, &TokenAmount::from_whole(10_000));
    let (owner, worker) = (addrs[0], addrs[0]);
    let (id_addr, _) = create_miner(
        v,
        &owner,
        &worker,
        seal_proof.registered_window_post_proof().unwrap(),
        &TokenAmount::from_whole(10_000),
    );
    v.set_epoch(200);

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
    #[allow(unused_variables)]
    let mut pre_committed_count = 0;

    let vec_onboarding = vec![
        Onboarding::new(1, 20, 12, 0, 0),
        Onboarding::new(PRE_COMMIT_CHALLENGE_DELAY + 1, 0, 0, 8, MAX_AGGREGATED_SECTORS as usize),
        Onboarding::new(1, 0, 0, 8, 4),
        Onboarding::new(1, 10, 4, 0, 0),
        Onboarding::new(PRE_COMMIT_CHALLENGE_DELAY + 1, 0, 0, 24, 10),
    ];

    let mut precommmits: Vec<SectorPreCommitOnChainInfo> = vec![];

    for item in vec_onboarding {
        let epoch = v.epoch();
        v.set_epoch(epoch + item.epoch_delay);

        if item.pre_commit_sector_count > 0 {
            let mut new_precommits = precommit_sectors_v2(
                v,
                item.pre_commit_sector_count,
                item.pre_commit_batch_size,
                vec![],
                &worker,
                &id_addr,
                *seal_proof,
                next_sector_no,
                next_sector_no == 0,
                None,
            );
            precommmits.append(&mut new_precommits);
            next_sector_no += item.pre_commit_sector_count as u64;
            pre_committed_count += item.pre_commit_sector_count;
        }

        if item.prove_commit_sector_count > 0 {
            let to_prove = precommmits[..item.prove_commit_sector_count].to_vec();
            precommmits = precommmits[item.prove_commit_sector_count..].to_vec();
            prove_commit_sectors(v, &worker, &id_addr, to_prove, item.prove_commit_aggregate_size);
            proven_count += item.prove_commit_sector_count;
        }
    }

    let (dline_info, p_idx) = advance_to_proving_deadline(v, &id_addr, 0);

    // submit post
    let st: MinerState = get_state(v, &id_addr).unwrap();
    let sector = st.get_sector(&DynBlockstore::wrap(v.blockstore()), 0).unwrap().unwrap();
    let mut new_power = power_for_sector(seal_proof.sector_size().unwrap(), &sector);
    new_power.raw *= proven_count;
    new_power.qa *= proven_count;

    submit_windowed_post(v, &worker, &id_addr, dline_info, p_idx, Some(new_power.clone()));

    let balances = miner_balance(v, &id_addr);
    assert!(balances.initial_pledge.is_positive());

    let network_stats = get_network_stats(v);
    let sector_size = seal_proof.sector_size().unwrap() as u64;
    assert_eq!(
        network_stats.total_bytes_committed,
        BigInt::from(sector_size * proven_count as u64)
    );
    assert!(network_stats.total_pledge_collateral.is_positive());

    cron_tick(v);

    let network_stats = get_network_stats(v);
    let sector_size = seal_proof.sector_size().unwrap() as u64;
    assert_eq!(
        network_stats.total_bytes_committed,
        BigInt::from(sector_size * proven_count as u64)
    );
    assert!(network_stats.total_pledge_collateral.is_positive());

    expect_invariants(
        v,
        &Policy::default(),
        &[invariant_failure_patterns::REWARD_STATE_EPOCH_MISMATCH.to_owned()],
        None,
    );
}
