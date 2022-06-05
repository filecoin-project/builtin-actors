use cid::Cid;
use fil_actor_init::ExecReturn;
use fil_actor_miner::{
    aggregate_pre_commit_network_fee, max_prove_commit_duration, Method as MinerMethod,
    PreCommitSectorBatchParams, ProveCommitAggregateParams, ProveCommitSectorParams,
    SectorPreCommitInfo, SectorPreCommitOnChainInfo, State as MinerState,
};
use fil_actor_multisig::{
    compute_proposal_hash, Method as MsigMethod, ProposeParams, RemoveSignerParams,
    State as MsigState, SwapSignerParams, Transaction, TxnID, TxnIDParams,
};
use fil_actor_market::{Method as MarketMethod};
use fil_actor_power::{CreateMinerParams, CreateMinerReturn, Method as PowerMethod};
use fil_actor_reward::Method as RewardMethod;
use fil_actor_cron::Method as CronMethod;
use fil_actors_runtime::cbor::serialize;
use fil_actors_runtime::runtime::{DomainSeparationTag, Policy, Runtime, RuntimePolicy};
use fil_actors_runtime::{
    make_map_with_root, INIT_ACTOR_ADDR, REWARD_ACTOR_ADDR, STORAGE_POWER_ACTOR_ADDR,
    SYSTEM_ACTOR_ADDR, STORAGE_MARKET_ACTOR_ADDR, CRON_ACTOR_ADDR,
};
use fil_actors_runtime::{test_utils::*, BURNT_FUNDS_ACTOR_ADDR};
use fvm_ipld_blockstore::MemoryBlockstore;
use fvm_ipld_encoding::{BytesDe, RawBytes};
use fvm_shared::address::Address;
use fvm_shared::bigint::Zero;
use fvm_shared::clock::{ChainEpoch, QuantSpec, NO_QUANTIZATION};
use fvm_shared::commcid::{FIL_COMMITMENT_SEALED, FIL_COMMITMENT_UNSEALED};
use fvm_shared::econ::TokenAmount;
use fvm_shared::error::ExitCode;
use fvm_shared::sector::{RegisteredPoStProof, RegisteredSealProof, SectorNumber};
use fvm_shared::METHOD_SEND;
use integer_encoding::VarInt;
use multihash::derive::Multihash;
use multihash::MultihashDigest;
use num_traits::sign::Signed;
use num_traits::FromPrimitive;
use std::collections::HashSet;
use std::iter::FromIterator;
use test_vm::util::{apply_code, apply_ok, create_accounts};
use test_vm::{ExpectInvocation, TEST_FAUCET_ADDR, VM};

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
    let precommits =
        precommit_sectors(&mut v, 1, 1, worker, id_addr, seal_proof, sector_number, true, None);

    let balances = v.get_miner_balance(id_addr);
    assert!(!balances.pre_commit_deposit.is_negative());

    let prove_time =
        v.get_epoch() + max_prove_commit_duration(&Policy::default(), seal_proof).unwrap();
    let v = v.with_epoch(prove_time);

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
        subinvocs: Some(vec![ExpectInvocation{to: *STORAGE_MARKET_ACTOR_ADDR, method: MarketMethod::ComputeDataCommitment as u64, ..Default::default()},ExpectInvocation{to: *STORAGE_POWER_ACTOR_ADDR, method: PowerMethod::SubmitPoRepForBulkVerify as u64, ..Default::default()}]),
        ..Default::default()
    }.matches(v.take_invocations().last().unwrap());
    let res = v.apply_message(*SYSTEM_ACTOR_ADDR, *CRON_ACTOR_ADDR, TokenAmount::zero(), CronMethod::EpochTick as u64, RawBytes::default()).unwrap();
    assert_eq!(ExitCode::OK, res.code);

}

fn create_miner(
    v: &mut VM,
    owner: Address,
    worker: Address,
    post_proof_type: RegisteredPoStProof,
    balance: TokenAmount,
) -> (Address, Address) {
    let multiaddrs = vec![BytesDe("multiaddr".as_bytes().to_vec())];
    let peer_id = "miner".as_bytes().to_vec();
    let params = CreateMinerParams {
        owner,
        worker: worker,
        window_post_proof_type: post_proof_type,
        peer: peer_id.clone(),
        multiaddrs: multiaddrs.clone(),
    };

    let res: CreateMinerReturn = v
        .apply_message(
            owner,
            *STORAGE_POWER_ACTOR_ADDR,
            balance,
            PowerMethod::CreateMiner as u64,
            params.clone(),
        )
        .unwrap()
        .ret
        .deserialize()
        .unwrap();
    (res.id_address, res.robust_address)
}

fn precommit_sectors(
    v: &mut VM,
    count: u64,
    batch_size: i64,
    worker: Address,
    maddr: Address,
    seal_proof: RegisteredSealProof,
    sector_number_base: SectorNumber,
    expect_cron_enroll: bool,
    exp: Option<ChainEpoch>,
) -> Vec<SectorPreCommitOnChainInfo> {
    let invocs_common = || -> Vec<ExpectInvocation> {
        vec![
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
        ]
    };
    let invoc_first = || -> ExpectInvocation {
        ExpectInvocation {
            to: *STORAGE_POWER_ACTOR_ADDR,
            method: PowerMethod::EnrollCronEvent as u64,
            ..Default::default()
        }
    };
    let invoc_net_fee = |fee: TokenAmount| -> ExpectInvocation {
        ExpectInvocation {
            to: *BURNT_FUNDS_ACTOR_ADDR,
            method: METHOD_SEND,
            value: Some(fee),
            ..Default::default()
        }
    };
    let expiration = match exp {
        None => {
            v.get_epoch()
                + Policy::default().min_sector_expiration
                + max_prove_commit_duration(&Policy::default(), seal_proof).unwrap()
        }
        Some(e) => e,
    };

    let mut sector_idx = 0u64;
    while sector_idx < count {
        let msg_sector_idx_base = sector_idx;
        let mut invocs = invocs_common();

        let mut param_sectors = Vec::<SectorPreCommitInfo>::new();
        let mut j = 0;
        while j < batch_size && sector_idx < count {
            let sector_number = sector_number_base + sector_idx;
            param_sectors.push(SectorPreCommitInfo {
                seal_proof,
                sector_number,
                sealed_cid: make_sealed_cid(format!("sn: {}", sector_number).as_bytes()),
                seal_rand_epoch: v.get_epoch() - 1,
                deal_ids: vec![],
                expiration,
                ..Default::default()
            });
            sector_idx += 1;
            j += 1;
        }
        if param_sectors.len() > 1 {
            invocs.push(invoc_net_fee(aggregate_pre_commit_network_fee(
                param_sectors.len() as i64,
                &TokenAmount::zero(),
            )));
        }
        if expect_cron_enroll && msg_sector_idx_base == 0 {
            invocs.push(invoc_first());
        }
        apply_ok(
            v,
            worker,
            maddr,
            TokenAmount::zero(),
            MinerMethod::PreCommitSectorBatch as u64,
            PreCommitSectorBatchParams { sectors: param_sectors.clone() },
        );
        let expect = ExpectInvocation {
            to: maddr,
            method: MinerMethod::PreCommitSectorBatch as u64,
            params: Some(
                serialize(
                    &PreCommitSectorBatchParams { sectors: param_sectors },
                    "precommit batch params",
                )
                .unwrap(),
            ),
            subinvocs: Some(invocs),
            ..Default::default()
        };
        expect.matches(v.take_invocations().last().unwrap())
    }
    // extract chain state
    let mstate = v.get_state::<MinerState>(maddr).unwrap();
    (0..count)
        .map(|i| mstate.get_precommitted_sector(v.store, sector_number_base + i).unwrap().unwrap())
        .collect()
}
