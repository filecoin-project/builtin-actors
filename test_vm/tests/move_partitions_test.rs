use fvm_ipld_blockstore::MemoryBlockstore;
use fvm_ipld_encoding::ipld_block::IpldBlock;
use fvm_shared::address::Address;
use fvm_shared::bigint::Zero;
use fvm_shared::econ::TokenAmount;
use fvm_shared::error::ExitCode;
use fvm_shared::sector::{RegisteredSealProof, SectorNumber};

use fil_actor_cron::Method as CronMethod;
use fil_actor_market::Method as MarketMethod;
use fil_actor_miner::{
    power_for_sector, DeadlineInfo, Method as MinerMethod, MovePartitionsParams,
    ProveCommitSectorParams, State as MinerState,
};

use fil_actor_power::{Method as PowerMethod, State as PowerState};
use fil_actors_integration_tests::expects::Expect;
use fil_actors_integration_tests::util::{
    advance_by_deadline_to_epoch, advance_to_proving_deadline, assert_invariants, create_accounts,
    create_miner, cron_tick, get_network_stats, make_bitfield, miner_balance, precommit_sectors,
    submit_windowed_post,
};
use fil_actors_runtime::runtime::Policy;
use fil_actors_runtime::{
    CRON_ACTOR_ADDR, CRON_ACTOR_ID, STORAGE_MARKET_ACTOR_ADDR, STORAGE_POWER_ACTOR_ADDR,
    STORAGE_POWER_ACTOR_ID, SYSTEM_ACTOR_ADDR,
};
use test_vm::TestVM;
use vm_api::trace::ExpectInvocation;
use vm_api::util::{apply_ok, get_state, DynBlockstore};
use vm_api::VM;

#[test]
fn move_partitions_success() {
    let store = MemoryBlockstore::new();
    let (v, miner, sector) = setup(&store);

    submit_post_succeeds_test(&v, miner.clone(), sector);

    let prove_time = v.epoch() + Policy::default().wpost_dispute_window;
    advance_by_deadline_to_epoch(&v, &miner.miner_id, prove_time);

    let move_params = MovePartitionsParams {
        orig_deadline: 0,
        dest_deadline: 47,
        partitions: make_bitfield(&[0u64]),
    };
    let prove_params_ser = IpldBlock::serialize_cbor(&move_params).unwrap();
    apply_ok(
        &v,
        &miner.worker,
        &miner.miner_robust,
        &TokenAmount::zero(),
        MinerMethod::MovePartitions as u64,
        Some(move_params),
    );
    ExpectInvocation {
        from: miner.worker.id().unwrap(),
        to: miner.miner_id,
        method: MinerMethod::MovePartitions as u64,
        params: Some(prove_params_ser),
        subinvocs: Some(vec![]),
        ..Default::default()
    }
    .matches(v.take_invocations().last().unwrap());

    cron_tick(&v);
    v.set_epoch(v.epoch() + 1);
    assert_invariants(&v, &Policy::default());
}

fn submit_post_succeeds_test(v: &dyn VM, miner_info: MinerInfo, sector_info: SectorInfo) {
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

    assert_invariants(v, &Policy::default());
}

struct SectorInfo {
    number: SectorNumber,
    deadline_info: DeadlineInfo,
    partition_index: u64,
}

#[derive(Clone)]
struct MinerInfo {
    seal_proof: RegisteredSealProof,
    worker: Address,
    miner_id: Address,
    miner_robust: Address,
}

fn setup(store: &'_ MemoryBlockstore) -> (TestVM<MemoryBlockstore>, MinerInfo, SectorInfo) {
    let v = TestVM::<MemoryBlockstore>::new_with_singletons(store);
    let addrs = create_accounts(&v, 1, &TokenAmount::from_whole(10_000));
    let seal_proof = RegisteredSealProof::StackedDRG32GiBV1P1;
    let (owner, worker) = (addrs[0], addrs[0]);
    let (id_addr, robust_addr) = create_miner(
        &v,
        &owner,
        &worker,
        seal_proof.registered_window_post_proof().unwrap(),
        &TokenAmount::from_whole(10_000),
    );
    v.set_epoch(200);

    // precommit and advance to prove commit time
    let sector_number: SectorNumber = 100;
    precommit_sectors(&v, 1, 1, &worker, &id_addr, seal_proof, sector_number, true, None);

    let balances = miner_balance(&v, &id_addr);
    assert!(balances.pre_commit_deposit.is_positive());

    let prove_time = v.epoch() + Policy::default().pre_commit_challenge_delay + 1;
    advance_by_deadline_to_epoch(&v, &id_addr, prove_time);

    // prove commit, cron, advance to post time
    let prove_params = ProveCommitSectorParams { sector_number, proof: vec![] };
    let prove_params_ser = IpldBlock::serialize_cbor(&prove_params).unwrap();
    apply_ok(
        &v,
        &worker,
        &robust_addr,
        &TokenAmount::zero(),
        MinerMethod::ProveCommitSector as u64,
        Some(prove_params),
    );
    ExpectInvocation {
        from: worker.id().unwrap(),
        to: id_addr,
        method: MinerMethod::ProveCommitSector as u64,
        params: Some(prove_params_ser),
        subinvocs: Some(vec![Expect::power_submit_porep(id_addr.id().unwrap())]),
        ..Default::default()
    }
    .matches(v.take_invocations().last().unwrap());
    let res = v
        .execute_message(
            &SYSTEM_ACTOR_ADDR,
            &CRON_ACTOR_ADDR,
            &TokenAmount::zero(),
            CronMethod::EpochTick as u64,
            None,
        )
        .unwrap();
    assert_eq!(ExitCode::OK, res.code);
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
                    ExpectInvocation {
                        from: STORAGE_POWER_ACTOR_ID,
                        to: id_addr,
                        method: MinerMethod::ConfirmSectorProofsValid as u64,
                        subinvocs: Some(vec![Expect::power_update_pledge(
                            id_addr.id().unwrap(),
                            None,
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
    // pcd is released ip is added
    let balances = miner_balance(&v, &id_addr);
    assert!(balances.initial_pledge.is_positive());
    assert!(balances.pre_commit_deposit.is_zero());

    // power unproven so network stats are the same

    let network_stats = get_network_stats(&v);
    assert!(network_stats.total_bytes_committed.is_zero());
    assert!(network_stats.total_pledge_collateral.is_positive());

    let (deadline_info, partition_index) = advance_to_proving_deadline(&v, &id_addr, sector_number);
    (
        v,
        MinerInfo { seal_proof, worker, miner_id: id_addr, miner_robust: robust_addr },
        SectorInfo { number: sector_number, deadline_info, partition_index },
    )
}
