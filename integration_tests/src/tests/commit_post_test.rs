use cid::Cid;
use export_macro::vm_test;
use fil_actors_runtime::runtime::policy_constants::MAX_SECTOR_NUMBER;
use fvm_ipld_bitfield::BitField;
use fvm_ipld_encoding::ipld_block::IpldBlock;
use fvm_shared::address::Address;
use fvm_shared::bigint::Zero;
use fvm_shared::econ::TokenAmount;
use fvm_shared::error::ExitCode;
use fvm_shared::randomness::Randomness;
use fvm_shared::sector::{PoStProof, RegisteredSealProof, SectorNumber};

use crate::expects::Expect;
use crate::util::{
    advance_by_deadline_to_epoch, advance_to_proving_deadline, assert_invariants, create_accounts,
    create_miner, cron_tick, expect_invariants, get_network_stats, invariant_failure_patterns,
    miner_balance, miner_prove_sector, precommit_sectors_v2, submit_windowed_post,
};
use crate::TEST_VM_RAND_ARRAY;
use fil_actor_cron::Method as CronMethod;
use fil_actor_market::Method as MarketMethod;
use fil_actor_miner::{
    max_prove_commit_duration, power_for_sector, DeadlineInfo, Method as MinerMethod,
    PoStPartition, ProveCommitAggregateParams, State as MinerState, SubmitWindowedPoStParams,
};
use fil_actor_power::{Method as PowerMethod, State as PowerState};
use fil_actors_runtime::runtime::Policy;
use fil_actors_runtime::{
    CRON_ACTOR_ADDR, CRON_ACTOR_ID, STORAGE_MARKET_ACTOR_ADDR, STORAGE_POWER_ACTOR_ADDR,
    STORAGE_POWER_ACTOR_ID,
};
use vm_api::trace::{EmittedEvent, ExpectInvocation};
use vm_api::util::{apply_code, apply_ok, get_state, DynBlockstore};
use vm_api::VM;

struct SectorInfo {
    number: SectorNumber,
    deadline_info: DeadlineInfo,
    partition_index: u64,
}

struct MinerInfo {
    seal_proof: RegisteredSealProof,
    _owner: Address,
    worker: Address,
    miner_id: Address,
    _miner_robust: Address,
}

fn setup(v: &dyn VM) -> (MinerInfo, SectorInfo) {
    let addrs = create_accounts(v, 1, &TokenAmount::from_whole(10_000));
    let seal_proof = RegisteredSealProof::StackedDRG32GiBV1P1;
    let (owner, worker) = (addrs[0], addrs[0]);
    let (id_addr, robust_addr) = create_miner(
        v,
        &owner,
        &worker,
        seal_proof.registered_window_post_proof().unwrap(),
        &TokenAmount::from_whole(10_000),
    );
    v.set_epoch(200);

    // precommit and advance to prove commit time
    let sector_number: SectorNumber = 100;
    let _ = precommit_sectors_v2(
        v,
        1,
        1,
        vec![],
        &worker,
        &id_addr,
        seal_proof,
        sector_number,
        true,
        None,
    );

    let balances = miner_balance(v, &id_addr);
    assert!(balances.pre_commit_deposit.is_positive());

    let prove_time = v.epoch() + Policy::default().pre_commit_challenge_delay + 1;
    advance_by_deadline_to_epoch(v, &id_addr, prove_time);

    // prove commit, cron, advance to post time
    miner_prove_sector(v, &worker, &id_addr, sector_number, vec![]);
    cron_tick(v);

    ExpectInvocation {
        to: CRON_ACTOR_ADDR,
        method: CronMethod::EpochTick as u64,
        subinvocs: Some(vec![
            ExpectInvocation {
                from: CRON_ACTOR_ID,
                to: STORAGE_POWER_ACTOR_ADDR,
                method: PowerMethod::OnEpochTickEnd as u64,
                subinvocs: Some(vec![
                    Expect::reward_this_epoch(STORAGE_POWER_ACTOR_ID),
                    Expect::reward_update_kpi(),
                ]),
                events: Some(vec![]),
                ..Default::default()
            },
            ExpectInvocation {
                from: CRON_ACTOR_ID,
                to: STORAGE_MARKET_ACTOR_ADDR,
                method: MarketMethod::CronTick as u64,
                events: Some(vec![]),
                ..Default::default()
            },
        ]),
        ..Default::default()
    }
    .matches(v.take_invocations().last().unwrap());
    // pcd is released ip is added
    let balances = miner_balance(v, &id_addr);
    assert!(balances.initial_pledge.is_positive());
    assert!(balances.pre_commit_deposit.is_zero());

    // power unproven so network stats are the same

    let network_stats = get_network_stats(v);
    assert!(network_stats.total_bytes_committed.is_zero());
    assert!(network_stats.total_pledge_collateral.is_positive());

    let (deadline_info, partition_index) = advance_to_proving_deadline(v, &id_addr, sector_number);
    (
        MinerInfo {
            seal_proof,
            worker,
            _owner: owner,
            miner_id: id_addr,
            _miner_robust: robust_addr,
        },
        SectorInfo { number: sector_number, deadline_info, partition_index },
    )
}

#[vm_test]
pub fn submit_post_succeeds_test(v: &dyn VM) {
    let (miner_info, sector_info) = setup(v);
    // submit post
    let st: MinerState = get_state(v, &miner_info.miner_id).unwrap();
    let sector =
        st.get_sector(&DynBlockstore::wrap(v.blockstore()), sector_info.number).unwrap().unwrap();
    let sector_power = power_for_sector(miner_info.seal_proof.sector_size().unwrap(), &sector);
    submit_windowed_post(
        v,
        &miner_info.worker,
        &miner_info.miner_id,
        sector_info.deadline_info,
        sector_info.partition_index,
        Some(sector_power.clone()),
    );
    let balances = miner_balance(v, &miner_info.miner_id);
    assert!(balances.initial_pledge.is_positive());
    let p_st: PowerState = get_state(v, &STORAGE_POWER_ACTOR_ADDR).unwrap();
    assert_eq!(sector_power.raw, p_st.total_bytes_committed);

    assert_invariants(v, &Policy::default(), None);
}

#[vm_test]
pub fn skip_sector_test(v: &dyn VM) {
    let (miner_info, sector_info) = setup(v);
    // submit post, but skip the only sector in it
    let params = SubmitWindowedPoStParams {
        deadline: sector_info.deadline_info.index,
        partitions: vec![PoStPartition {
            index: sector_info.partition_index,
            skipped: BitField::try_from_bits([sector_info.number].iter().copied()).unwrap(),
        }],
        proofs: vec![PoStProof {
            post_proof: miner_info.seal_proof.registered_window_post_proof().unwrap(),
            proof_bytes: vec![],
        }],
        chain_commit_epoch: sector_info.deadline_info.challenge,
        chain_commit_rand: Randomness(TEST_VM_RAND_ARRAY.into()),
    };

    // PoSt is rejected for skipping all sectors.
    apply_code(
        v,
        &miner_info.worker,
        &miner_info.miner_id,
        &TokenAmount::zero(),
        MinerMethod::SubmitWindowedPoSt as u64,
        Some(params),
        ExitCode::USR_ILLEGAL_ARGUMENT,
    );

    // miner still has initial pledge
    let balances = miner_balance(v, &miner_info.miner_id);
    assert!(balances.initial_pledge.is_positive());

    // power unproven so network stats are the same
    let network_stats = get_network_stats(v);
    assert!(network_stats.total_bytes_committed.is_zero());
    assert!(network_stats.total_pledge_collateral.is_positive());
    assert_invariants(v, &Policy::default(), None)
}

#[vm_test]
pub fn missed_first_post_deadline_test(v: &dyn VM) {
    let (miner_info, sector_info) = setup(v);
    // move to proving period end
    v.set_epoch(sector_info.deadline_info.last());

    // Run cron to detect missing PoSt
    cron_tick(v);

    ExpectInvocation {
        to: CRON_ACTOR_ADDR,
        method: CronMethod::EpochTick as u64,
        params: None,
        subinvocs: Some(vec![
            ExpectInvocation {
                from: CRON_ACTOR_ID,
                to: STORAGE_POWER_ACTOR_ADDR,
                method: PowerMethod::OnEpochTickEnd as u64,
                subinvocs: Some(vec![
                    Expect::reward_this_epoch(STORAGE_POWER_ACTOR_ID),
                    ExpectInvocation {
                        from: STORAGE_POWER_ACTOR_ID,
                        to: miner_info.miner_id,
                        method: MinerMethod::OnDeferredCronEvent as u64,
                        subinvocs: Some(vec![Expect::power_enrol_cron(
                            miner_info.miner_id.id().unwrap(),
                        )]),
                        ..Default::default()
                    },
                    Expect::reward_update_kpi(),
                ]),
                ..Default::default()
            },
            ExpectInvocation {
                from: CRON_ACTOR_ID,
                to: STORAGE_MARKET_ACTOR_ADDR,
                method: MarketMethod::CronTick as u64,
                ..Default::default()
            },
        ]),
        ..Default::default()
    }
    .matches(v.take_invocations().last().unwrap());

    // power unproven so network stats are the same
    let network_stats = get_network_stats(v);
    assert!(network_stats.total_bytes_committed.is_zero());
    assert!(network_stats.total_pledge_collateral.is_positive());

    expect_invariants(
        v,
        &Policy::default(),
        &[invariant_failure_patterns::REWARD_STATE_EPOCH_MISMATCH.to_owned()],
        None,
    );
}

#[vm_test]
pub fn overdue_precommit_test(v: &dyn VM) {
    let policy = &Policy::default();
    let addrs = create_accounts(v, 1, &TokenAmount::from_whole(10_000));
    let seal_proof = RegisteredSealProof::StackedDRG32GiBV1P1;
    let (owner, worker) = (addrs[0], addrs[0]);
    let id_addr = create_miner(
        v,
        &owner,
        &worker,
        seal_proof.registered_window_post_proof().unwrap(),
        &TokenAmount::from_whole(10_000),
    )
    .0;
    v.set_epoch(200);

    // precommit and advance to prove commit time
    let sector_number: SectorNumber = 100;
    let precommit = precommit_sectors_v2(
        v,
        1,
        1,
        vec![],
        &worker,
        &id_addr,
        seal_proof,
        sector_number,
        true,
        None,
    )
    .first()
    .unwrap()
    .clone();

    let balances = miner_balance(v, &id_addr);
    assert!(balances.pre_commit_deposit.is_positive());

    let prove_time = v.epoch() + max_prove_commit_duration(policy, seal_proof).unwrap() + 1;
    advance_by_deadline_to_epoch(v, &id_addr, prove_time);

    //
    // overdue precommit
    //

    // advance time to precommit clean up epoch
    let cleanup_time = prove_time + policy.expired_pre_commit_clean_up_delay;
    let deadline_info = advance_by_deadline_to_epoch(v, &id_addr, cleanup_time);

    // advance one more deadline so precommit clean up is reached
    v.set_epoch(deadline_info.close);

    // run cron which should clean up precommit
    cron_tick(v);

    ExpectInvocation {
        to: CRON_ACTOR_ADDR,
        method: CronMethod::EpochTick as u64,
        params: None,
        subinvocs: Some(vec![
            ExpectInvocation {
                from: CRON_ACTOR_ID,
                to: STORAGE_POWER_ACTOR_ADDR,
                method: PowerMethod::OnEpochTickEnd as u64,
                subinvocs: Some(vec![
                    Expect::reward_this_epoch(STORAGE_POWER_ACTOR_ID),
                    ExpectInvocation {
                        from: STORAGE_POWER_ACTOR_ID,
                        to: id_addr,
                        method: MinerMethod::OnDeferredCronEvent as u64,
                        subinvocs: Some(vec![
                            // The call to burnt funds indicates the overdue precommit has been penalized
                            Expect::burn(id_addr.id().unwrap(), Some(precommit.pre_commit_deposit)),
                            // No re-enrollment of cron because burning of PCD discontinues miner cron scheduling
                        ]),
                        ..Default::default()
                    },
                    Expect::reward_update_kpi(),
                ]),
                ..Default::default()
            },
            ExpectInvocation {
                from: CRON_ACTOR_ID,
                to: STORAGE_MARKET_ACTOR_ADDR,
                method: MarketMethod::CronTick as u64,
                ..Default::default()
            },
        ]),
        ..Default::default()
    }
    .matches(v.take_invocations().last().unwrap());

    let balances = miner_balance(v, &id_addr);
    assert!(balances.initial_pledge.is_zero());
    assert!(balances.pre_commit_deposit.is_zero());

    let network_stats = get_network_stats(v);
    assert!(network_stats.total_bytes_committed.is_zero());
    assert!(network_stats.total_pledge_collateral.is_zero());
    assert!(network_stats.total_raw_byte_power.is_zero());
    assert!(network_stats.total_quality_adj_power.is_zero());

    expect_invariants(
        v,
        &Policy::default(),
        &[invariant_failure_patterns::REWARD_STATE_EPOCH_MISMATCH.to_owned()],
        None,
    );
}

#[vm_test]
pub fn aggregate_bad_sector_number_test(v: &dyn VM) {
    let addrs = create_accounts(v, 1, &TokenAmount::from_whole(10_000));
    let seal_proof = RegisteredSealProof::StackedDRG32GiBV1P1;
    let (owner, worker) = (addrs[0], addrs[0]);
    let (id_addr, robust_addr) = create_miner(
        v,
        &owner,
        &worker,
        seal_proof.registered_window_post_proof().unwrap(),
        &TokenAmount::from_whole(10_000),
    );
    v.set_epoch(200);
    let policy = &Policy::default();

    //
    // precommit good sectors
    //

    // precommit and advance to prove commit time
    let sector_number: SectorNumber = 100;
    let mut precommited_sector_nos = BitField::try_from_bits(
        precommit_sectors_v2(
            v,
            4,
            256,
            vec![],
            &worker,
            &id_addr,
            seal_proof,
            sector_number,
            true,
            None,
        )
        .iter()
        .map(|info| info.info.sector_number),
    )
    .unwrap();

    //
    // attempt proving with invalid args
    //

    // advance time to max seal duration

    let prove_time = v.epoch() + policy.pre_commit_challenge_delay + 1;
    advance_by_deadline_to_epoch(v, &id_addr, prove_time);

    // construct invalid bitfield with a non-committed sector number > abi.MaxSectorNumber

    precommited_sector_nos.set(MAX_SECTOR_NUMBER + 1);

    let params = ProveCommitAggregateParams {
        sector_numbers: precommited_sector_nos.clone(),
        aggregate_proof: vec![].into(),
    };
    apply_code(
        v,
        &worker,
        &robust_addr,
        &TokenAmount::zero(),
        MinerMethod::ProveCommitAggregate as u64,
        Some(params),
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
pub fn aggregate_size_limits_test(v: &dyn VM) {
    let oversized_batch = 820;
    let addrs = create_accounts(v, 1, &TokenAmount::from_whole(100_000));
    let seal_proof = RegisteredSealProof::StackedDRG32GiBV1P1;
    let (owner, worker) = (addrs[0], addrs[0]);
    let (id_addr, robust_addr) = create_miner(
        v,
        &owner,
        &worker,
        seal_proof.registered_window_post_proof().unwrap(),
        &TokenAmount::from_whole(100_000),
    );
    v.set_epoch(200);
    let policy = &Policy::default();

    //
    // precommit good sectors
    //

    // precommit and advance to prove commit time
    let sector_number: SectorNumber = 100;
    let precommited_sector_nos = BitField::try_from_bits(
        precommit_sectors_v2(
            v,
            oversized_batch,
            256,
            vec![],
            &worker,
            &id_addr,
            seal_proof,
            sector_number,
            true,
            None,
        )
        .iter()
        .map(|info| info.info.sector_number),
    )
    .unwrap();

    //
    // attempt proving with invalid args
    //

    // advance time to max seal duration
    let prove_time = v.epoch() + policy.pre_commit_challenge_delay + 1;
    advance_by_deadline_to_epoch(v, &id_addr, prove_time);

    // Fail with too many sectors
    let params = ProveCommitAggregateParams {
        sector_numbers: precommited_sector_nos.clone(),
        aggregate_proof: vec![].into(),
    };
    apply_code(
        v,
        &worker,
        &robust_addr,
        &TokenAmount::zero(),
        MinerMethod::ProveCommitAggregate as u64,
        Some(params),
        ExitCode::USR_ILLEGAL_ARGUMENT,
    );

    // Fail with too few sectors
    let too_few_sector_nos_bf =
        precommited_sector_nos.slice(0, policy.min_aggregated_sectors - 1).unwrap();
    let params = ProveCommitAggregateParams {
        sector_numbers: too_few_sector_nos_bf,
        aggregate_proof: vec![].into(),
    };
    apply_code(
        v,
        &worker,
        &robust_addr,
        &TokenAmount::zero(),
        MinerMethod::ProveCommitAggregate as u64,
        Some(params),
        ExitCode::USR_ILLEGAL_ARGUMENT,
    );

    // Fail with proof too big
    let just_right_sectors_no_bf =
        precommited_sector_nos.slice(0, policy.max_aggregated_sectors).unwrap();
    let params = ProveCommitAggregateParams {
        sector_numbers: just_right_sectors_no_bf,
        aggregate_proof: vec![0; policy.max_aggregated_proof_size + 1].into(),
    };
    apply_code(
        v,
        &worker,
        &robust_addr,
        &TokenAmount::zero(),
        MinerMethod::ProveCommitAggregate as u64,
        Some(params),
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
pub fn aggregate_bad_sender_test(v: &dyn VM) {
    let addrs = create_accounts(v, 2, &TokenAmount::from_whole(10_000));
    let seal_proof = RegisteredSealProof::StackedDRG32GiBV1P1;
    let (owner, worker) = (addrs[0], addrs[0]);
    let (id_addr, robust_addr) = create_miner(
        v,
        &owner,
        &worker,
        seal_proof.registered_window_post_proof().unwrap(),
        &TokenAmount::from_whole(10_000),
    );
    v.set_epoch(200);
    let policy = &Policy::default();

    //
    // precommit good sectors
    //

    // precommit and advance to prove commit time
    let sector_number: SectorNumber = 100;
    let precommited_sector_nos = BitField::try_from_bits(
        precommit_sectors_v2(
            v,
            4,
            256,
            vec![],
            &worker,
            &id_addr,
            seal_proof,
            sector_number,
            true,
            None,
        )
        .iter()
        .map(|info| info.info.sector_number),
    )
    .unwrap();

    //
    // attempt proving with invalid args
    //

    // advance time to max seal duration

    let prove_time = v.epoch() + policy.pre_commit_challenge_delay + 1;
    advance_by_deadline_to_epoch(v, &id_addr, prove_time);

    let params = ProveCommitAggregateParams {
        sector_numbers: precommited_sector_nos,
        aggregate_proof: vec![].into(),
    };
    apply_code(
        v,
        &addrs[1],
        &robust_addr,
        &TokenAmount::zero(),
        MinerMethod::ProveCommitAggregate as u64,
        Some(params),
        ExitCode::USR_FORBIDDEN,
    );
    expect_invariants(
        v,
        &Policy::default(),
        &[invariant_failure_patterns::REWARD_STATE_EPOCH_MISMATCH.to_owned()],
        None,
    );
}

#[vm_test]
pub fn aggregate_one_precommit_expires_test(v: &dyn VM) {
    let addrs = create_accounts(v, 1, &TokenAmount::from_whole(10_000));
    let seal_proof = RegisteredSealProof::StackedDRG32GiBV1P1;
    let (owner, worker) = (addrs[0], addrs[0]);
    let worker_id = worker.id().unwrap();
    let (miner_addr, robust_addr) = create_miner(
        v,
        &owner,
        &worker,
        seal_proof.registered_window_post_proof().unwrap(),
        &TokenAmount::from_whole(10_000),
    );
    let miner_id = miner_addr.id().unwrap();
    v.set_epoch(200);
    let policy = &Policy::default();

    //
    // precommit sectors
    //

    let sector_number: SectorNumber = 100;

    // early precommit
    let early_precommit_time = v.epoch();
    let early_precommits = precommit_sectors_v2(
        v,
        1,
        256,
        vec![],
        &worker,
        &miner_addr,
        seal_proof,
        sector_number,
        true,
        None,
    );

    let early_pre_commit_invalid =
        early_precommit_time + max_prove_commit_duration(policy, seal_proof).unwrap() + 1;

    advance_by_deadline_to_epoch(v, &miner_addr, early_pre_commit_invalid);

    // later precommits

    let later_precommits = precommit_sectors_v2(
        v,
        3,
        256,
        vec![],
        &worker,
        &miner_addr,
        seal_proof,
        sector_number + 1,
        false,
        None,
    );

    let all_precommits = [early_precommits, later_precommits.clone()].concat();

    let sector_nos_bf =
        BitField::try_from_bits(all_precommits.iter().map(|info| info.info.sector_number)).unwrap();

    // Advance minimum epochs past later precommits for later commits to be valid

    let prove_time = v.epoch() + policy.pre_commit_challenge_delay + 1;
    let deadline_info = advance_by_deadline_to_epoch(v, &miner_addr, prove_time);
    advance_by_deadline_to_epoch(v, &miner_addr, deadline_info.close);

    // Assert that precommit should not yet be cleaned up. This makes fixing this test easier if parameters change.
    assert!(
        prove_time
            < early_precommit_time
                + max_prove_commit_duration(policy, seal_proof).unwrap()
                + policy.expired_pre_commit_clean_up_delay
    );

    // Assert that we have a valid aggregate batch size
    let agg_setors_count = sector_nos_bf.len();
    assert!(
        agg_setors_count >= policy.min_aggregated_sectors
            && agg_setors_count < policy.max_aggregated_sectors
    );

    let prove_params = ProveCommitAggregateParams {
        sector_numbers: sector_nos_bf,
        aggregate_proof: vec![].into(),
    };
    let prove_params_ser = IpldBlock::serialize_cbor(&prove_params).unwrap();
    apply_ok(
        v,
        &worker,
        &robust_addr,
        &TokenAmount::zero(),
        MinerMethod::ProveCommitAggregate as u64,
        Some(prove_params),
    );

    let events: Vec<EmittedEvent> = later_precommits
        .iter()
        .map(|info| {
            let pieces: Vec<(Cid, u64)> = vec![];
            let unsealed_cid = info.info.unsealed_cid.0;
            Expect::build_sector_activation_event(
                "sector-activated",
                miner_id,
                info.info.sector_number,
                unsealed_cid,
                &pieces,
            )
        })
        .collect();

    ExpectInvocation {
        from: worker_id,
        to: miner_addr,
        method: MinerMethod::ProveCommitAggregate as u64,
        params: Some(prove_params_ser),
        subinvocs: Some(vec![
            Expect::reward_this_epoch(miner_id),
            Expect::power_current_total(miner_id),
            Expect::power_update_pledge(miner_id, None),
            Expect::burn(miner_id, None),
        ]),
        events: Some(events),
        ..Default::default()
    }
    .matches(v.take_invocations().last().unwrap());

    let balances = miner_balance(v, &miner_addr);
    assert!(balances.initial_pledge.is_positive());
    assert!(balances.pre_commit_deposit.is_positive());

    expect_invariants(
        v,
        &Policy::default(),
        &[invariant_failure_patterns::REWARD_STATE_EPOCH_MISMATCH.to_owned()],
        None,
    );
}
