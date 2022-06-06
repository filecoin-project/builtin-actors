use fil_actor_cron::Method as CronMethod;
use fil_actor_market::Method as MarketMethod;
use fil_actor_miner::{
    aggregate_pre_commit_network_fee, max_prove_commit_duration,
    new_deadline_info_from_offset_and_epoch, power_for_sector, DeadlineInfo, Method as MinerMethod,
    PoStPartition, PowerPair, PreCommitSectorBatchParams, ProveCommitSectorParams,
    SectorPreCommitInfo, SectorPreCommitOnChainInfo, State as MinerState, SubmitWindowedPoStParams,
};
use fil_actor_power::{
    CreateMinerParams, CreateMinerReturn, Method as PowerMethod, State as PowerState,
    UpdateClaimedPowerParams,
};
use fil_actor_reward::Method as RewardMethod;
use fil_actors_runtime::cbor::serialize;
use fil_actors_runtime::runtime::Policy;
use fil_actors_runtime::{test_utils::*, BURNT_FUNDS_ACTOR_ADDR};
use fil_actors_runtime::{
    CRON_ACTOR_ADDR, REWARD_ACTOR_ADDR, STORAGE_MARKET_ACTOR_ADDR, STORAGE_POWER_ACTOR_ADDR,
    SYSTEM_ACTOR_ADDR,
};
use fvm_ipld_bitfield::BitField;
use fvm_ipld_blockstore::MemoryBlockstore;
use fvm_ipld_encoding::{BytesDe, RawBytes};
use fvm_shared::address::Address;
use fvm_shared::bigint::Zero;
use fvm_shared::clock::ChainEpoch;
use fvm_shared::econ::TokenAmount;
use fvm_shared::error::ExitCode;
use fvm_shared::randomness::Randomness;
use fvm_shared::sector::{
    PoStProof, RegisteredPoStProof, RegisteredSealProof, SectorNumber, StoragePower,
};
use fvm_shared::METHOD_SEND;
use num_traits::sign::Signed;
use test_vm::util::{apply_ok, create_accounts};
use test_vm::{ExpectInvocation, TEST_VM_RAND_STRING, VM};

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
    let partitions = vec![PoStPartition {
        index: p_idx,
        skipped: fvm_ipld_bitfield::UnvalidatedBitField::Validated(BitField::new()),
    }];
    let sector_power = power_for_sector(seal_proof.sector_size().unwrap(), &sector);
    submit_windowed_post(&v, worker, id_addr, dline_info, partitions, sector_power.clone());
    let balances = v.get_miner_balance(id_addr);
    assert!(balances.initial_pledge.is_positive());
    let p_st = v.get_state::<PowerState>(*STORAGE_POWER_ACTOR_ADDR).unwrap();
    assert_eq!(
        sector_power.raw,
        p_st.total_bytes_committed
    );
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
            params,
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

fn advance_by_deadline_to_epoch<'bs>(
    v: VM<'bs>,
    maddr: Address,
    e: ChainEpoch,
) -> (VM<'bs>, DeadlineInfo) {
    advance_by_deadline(v, maddr, |dline_info| dline_info.close <= e)
}

fn advance_by_deadline_to_index<'bs>(
    v: VM<'bs>,
    maddr: Address,
    i: u64,
) -> (VM<'bs>, DeadlineInfo) {
    advance_by_deadline(v, maddr, |dline_info| dline_info.index != i)
}

fn advance_to_proving_deadline<'bs>(
    v: VM<'bs>,
    maddr: Address,
    s: SectorNumber,
) -> (DeadlineInfo, u64, VM<'bs>) {
    let (d, p) = sector_deadline(&v, maddr, s);
    let (v, dline_info) = advance_by_deadline_to_index(v, maddr, d);
    let v = v.with_epoch(dline_info.open);
    (dline_info, p, v)
}

fn advance_by_deadline<'bs, F>(mut v: VM<'bs>, maddr: Address, more: F) -> (VM<'bs>, DeadlineInfo)
where
    F: Fn(DeadlineInfo) -> bool,
{
    loop {
        let dline_info = miner_dline_info(&v, maddr);
        if !more(dline_info) {
            return (v, dline_info);
        }
        v = v.with_epoch(dline_info.last());

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
        let next = v.get_epoch() + 1;
        v = v.with_epoch(next);
    }
}

fn miner_dline_info(v: &VM, m: Address) -> DeadlineInfo {
    let st = v.get_state::<MinerState>(m).unwrap();
    new_deadline_info_from_offset_and_epoch(
        &Policy::default(),
        st.proving_period_start,
        v.get_epoch(),
    )
}

fn sector_deadline(v: &VM, m: Address, s: SectorNumber) -> (u64, u64) {
    let st = v.get_state::<MinerState>(m).unwrap();
    st.find_sector(&Policy::default(), v.store, s).unwrap()
}

fn submit_windowed_post(
    v: &VM,
    worker: Address,
    maddr: Address,
    dline_info: DeadlineInfo,
    partitions: Vec<PoStPartition>,
    sector: PowerPair,
) {
    let params = SubmitWindowedPoStParams {
        deadline: dline_info.index,
        partitions,
        proofs: vec![PoStProof {
            post_proof: RegisteredPoStProof::StackedDRGWindow32GiBV1,
            proof_bytes: vec![],
        }],
        chain_commit_epoch: dline_info.challenge,
        chain_commit_rand: Randomness(TEST_VM_RAND_STRING.to_owned().into_bytes()),
    };
    apply_ok(
        &v,
        worker,
        maddr,
        TokenAmount::zero(),
        MinerMethod::SubmitWindowedPoSt as u64,
        params,
    );

    let update_power_params = serialize(
        &UpdateClaimedPowerParams { raw_byte_delta: sector.raw, quality_adjusted_delta: sector.qa },
        "update claim params",
    )
    .unwrap();
    ExpectInvocation {
        to: maddr,
        method: MinerMethod::SubmitWindowedPoSt as u64,
        subinvocs: Some(vec![ExpectInvocation {
            to: *STORAGE_POWER_ACTOR_ADDR,
            method: PowerMethod::UpdateClaimedPower as u64,
            params: Some(update_power_params),
            ..Default::default()
        }]),
        ..Default::default()
    }
    .matches(v.take_invocations().last().unwrap());
}
