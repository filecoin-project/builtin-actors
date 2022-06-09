use fil_actor_cron::Method as CronMethod;
use fil_actor_market::Method as MarketMethod;
use fil_actor_miner::{
    max_prove_commit_duration, power_for_sector, Method as MinerMethod,
    ProveCommitSectorParams, State as MinerState,
};
use fil_actor_power::{Method as PowerMethod, State as PowerState};
use fil_actor_reward::Method as RewardMethod;
use fil_actors_runtime::cbor::serialize;
use fil_actors_runtime::runtime::Policy;
use fil_actors_runtime::{
    CRON_ACTOR_ADDR, REWARD_ACTOR_ADDR, STORAGE_MARKET_ACTOR_ADDR, STORAGE_POWER_ACTOR_ADDR,
    SYSTEM_ACTOR_ADDR,
};
use fvm_ipld_blockstore::MemoryBlockstore;
use fvm_ipld_encoding::RawBytes;
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
fn commit_post_flow_happy_path() {
    let store = MemoryBlockstore::new();
    let mut v = VM::new_with_singletons(&store);
    let addrs = create_accounts(&v, 2, TokenAmount::from(10_000e18 as i128));
    let seal_proof = RegisteredSealProof::StackedDRG32GiBV1P1;
    let (owner, worker) = (addrs[0], addrs[0]);
    let (id_addr, robust_addr) = create_miner(
        &mut v,
        owner,
        worker,
        seal_proof.registered_window_post_proof().unwrap(),
        TokenAmount::from(10_000e18 as i128),
    );
    let mut v = v.with_epoch(200);

    // precommit and advance to prove commit time
    let sector_number: SectorNumber = 100;
    precommit_sectors(&mut v, 1, 1, worker, id_addr, seal_proof, sector_number, true, None);

    let balances = v.get_miner_balance(id_addr);
    assert!(balances.pre_commit_deposit.is_positive());

    let prove_time =
        v.get_epoch() + max_prove_commit_duration(&Policy::default(), seal_proof).unwrap();
    let v = advance_by_deadline_to_epoch(v, id_addr, prove_time).0;

    // prove commit, cron, advance to post time
    let prove_params = ProveCommitSectorParams { sector_number, proof: vec![] };
    let prove_params_ser = serialize(&prove_params, "commit params").unwrap();
    apply_ok(
        &v,
        worker,
        robust_addr,
        TokenAmount::zero(),
        MinerMethod::ProveCommitSector as u64,
        prove_params,
    );
    ExpectInvocation {
        to: id_addr,
        method: MinerMethod::ProveCommitSector as u64,
        params: Some(prove_params_ser),
        subinvocs: Some(vec![
            ExpectInvocation {
                to: *STORAGE_MARKET_ACTOR_ADDR,
                method: MarketMethod::ComputeDataCommitment as u64,
                ..Default::default()
            },
            ExpectInvocation {
                to: *STORAGE_POWER_ACTOR_ADDR,
                method: PowerMethod::SubmitPoRepForBulkVerify as u64,
                ..Default::default()
            },
        ]),
        ..Default::default()
    }
    .matches(v.take_invocations().last().unwrap());
    let res = v
        .apply_message(
            *SYSTEM_ACTOR_ADDR,
            *CRON_ACTOR_ADDR,
            TokenAmount::zero(),
            CronMethod::EpochTick as u64,
            RawBytes::default(),
        )
        .unwrap();
    assert_eq!(ExitCode::OK, res.code);
    ExpectInvocation {
        to: *CRON_ACTOR_ADDR,
        method: CronMethod::EpochTick as u64,
        subinvocs: Some(vec![
            ExpectInvocation {
                to: *STORAGE_POWER_ACTOR_ADDR,
                method: PowerMethod::OnEpochTickEnd as u64,
                subinvocs: Some(vec![
                    ExpectInvocation {
                        to: *REWARD_ACTOR_ADDR,
                        method: RewardMethod::ThisEpochReward as u64,
                        ..Default::default()
                    },
                    ExpectInvocation {
                        to: id_addr,
                        method: MinerMethod::ConfirmSectorProofsValid as u64,
                        subinvocs: Some(vec![ExpectInvocation {
                            to: *STORAGE_POWER_ACTOR_ADDR,
                            method: PowerMethod::UpdatePledgeTotal as u64,
                            ..Default::default()
                        }]),
                        ..Default::default()
                    },
                    ExpectInvocation {
                        to: *REWARD_ACTOR_ADDR,
                        method: RewardMethod::UpdateNetworkKPI as u64,
                        ..Default::default()
                    },
                ]),
                ..Default::default()
            },
            ExpectInvocation {
                to: *STORAGE_MARKET_ACTOR_ADDR,
                method: MarketMethod::CronTick as u64,
                ..Default::default()
            },
        ]),
        ..Default::default()
    }
    .matches(v.take_invocations().last().unwrap());
    // pcd is released ip is added
    let balances = v.get_miner_balance(id_addr);
    assert!(balances.initial_pledge.is_positive());
    assert!(balances.pre_commit_deposit.is_zero());

    // power unproven so network stats are the same
    let p_st = v.get_state::<PowerState>(*STORAGE_POWER_ACTOR_ADDR).unwrap();
    assert!(p_st.total_bytes_committed.is_zero());
    assert!(p_st.total_pledge_collateral.is_positive());
    let (dline_info, p_idx, v) = advance_to_proving_deadline(v, id_addr, sector_number);

    // submit post
    let st = v.get_state::<MinerState>(id_addr).unwrap();
    let sector = st.get_sector(v.store, sector_number).unwrap().unwrap();
    let sector_power = power_for_sector(seal_proof.sector_size().unwrap(), &sector);
    submit_windowed_post(&v, worker, id_addr, dline_info, p_idx, sector_power.clone());
    let balances = v.get_miner_balance(id_addr);
    assert!(balances.initial_pledge.is_positive());
    let p_st = v.get_state::<PowerState>(*STORAGE_POWER_ACTOR_ADDR).unwrap();
    assert_eq!(sector_power.raw, p_st.total_bytes_committed);
}
