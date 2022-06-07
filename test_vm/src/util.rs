use crate::*;
use fil_actor_cron::Method as CronMethod;
use fil_actor_miner::{
    aggregate_pre_commit_network_fee, max_prove_commit_duration,
    new_deadline_info_from_offset_and_epoch, DeadlineInfo, Method as MinerMethod, PoStPartition,
    PowerPair, PreCommitSectorBatchParams, SectorPreCommitInfo, SectorPreCommitOnChainInfo,
    State as MinerState, SubmitWindowedPoStParams,
};
use fil_actor_power::{
    CreateMinerParams, CreateMinerReturn, Method as PowerMethod, UpdateClaimedPowerParams,
};
use fil_actor_reward::Method as RewardMethod;
use fvm_ipld_encoding::{BytesDe, Cbor, RawBytes};
use fvm_shared::address::{Address, BLS_PUB_LEN};
use fvm_shared::econ::TokenAmount;
use fvm_shared::error::ExitCode;
use fvm_shared::sector::{PoStProof, RegisteredPoStProof, RegisteredSealProof, SectorNumber};
use fvm_shared::{MethodNum, METHOD_SEND};
use rand::prelude::*;
use rand_chacha::ChaCha8Rng;

// Generate count addresses by seeding an rng
pub fn pk_addrs_from(seed: u64, count: u64) -> Vec<Address> {
    let mut seed_arr = [0u8; 32];
    for (i, b) in seed.to_ne_bytes().iter().enumerate() {
        seed_arr[i] = *b;
    }
    let mut rng = ChaCha8Rng::from_seed(seed_arr);
    (0..count).map(|_| new_bls_from_rng(&mut rng)).collect()
}

// Generate nice 32 byte arrays sampled uniformly at random based off of a u64 seed
fn new_bls_from_rng(rng: &mut ChaCha8Rng) -> Address {
    let mut bytes = [0u8; BLS_PUB_LEN];
    rng.fill_bytes(&mut bytes);
    Address::new_bls(&bytes).unwrap()
}

const ACCOUNT_SEED: u64 = 93837778;

pub fn create_accounts(v: &VM, count: u64, balance: TokenAmount) -> Vec<Address> {
    let pk_addrs = pk_addrs_from(ACCOUNT_SEED, count);
    // Send funds from faucet to pk address, creating account actor
    for pk_addr in pk_addrs.clone() {
        apply_ok(v, TEST_FAUCET_ADDR, pk_addr, balance.clone(), METHOD_SEND, RawBytes::default());
    }
    // Normalize pk address to return id address of account actor
    pk_addrs.iter().map(|&pk_addr| v.normalize_address(&pk_addr).unwrap()).collect()
}

pub fn apply_ok<C: Cbor>(
    v: &VM,
    from: Address,
    to: Address,
    value: TokenAmount,
    method: MethodNum,
    params: C,
) -> RawBytes {
    apply_code(v, from, to, value, method, params, ExitCode::OK)
}

pub fn apply_code<C: Cbor>(
    v: &VM,
    from: Address,
    to: Address,
    value: TokenAmount,
    method: MethodNum,
    params: C,
    code: ExitCode,
) -> RawBytes {
    let res = v.apply_message(from, to, value, method, params).unwrap();
    assert_eq!(code, res.code);
    res.ret
}

pub fn create_miner(
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
        worker,
        window_post_proof_type: post_proof_type,
        peer: peer_id,
        multiaddrs,
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

#[allow(clippy::too_many_arguments)]
pub fn precommit_sectors(
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

pub fn advance_by_deadline_to_epoch(v: VM, maddr: Address, e: ChainEpoch) -> (VM, DeadlineInfo) {
    advance_by_deadline(v, maddr, |dline_info| dline_info.close <= e)
}

pub fn advance_by_deadline_to_index(v: VM, maddr: Address, i: u64) -> (VM, DeadlineInfo) {
    advance_by_deadline(v, maddr, |dline_info| dline_info.index != i)
}

pub fn advance_to_proving_deadline(
    v: VM,
    maddr: Address,
    s: SectorNumber,
) -> (DeadlineInfo, u64, VM) {
    let (d, p) = sector_deadline(&v, maddr, s);
    let (v, dline_info) = advance_by_deadline_to_index(v, maddr, d);
    let v = v.with_epoch(dline_info.open);
    (dline_info, p, v)
}

fn advance_by_deadline<F>(mut v: VM, maddr: Address, more: F) -> (VM, DeadlineInfo)
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

pub fn submit_windowed_post(
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
    apply_ok(v, worker, maddr, TokenAmount::zero(), MinerMethod::SubmitWindowedPoSt as u64, params);

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
