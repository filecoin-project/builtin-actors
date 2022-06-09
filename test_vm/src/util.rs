use crate::*;
use fil_actor_cron::Method as CronMethod;
use fil_actor_market::{
    ClientDealProposal, DealProposal, Label, Method as MarketMethod, PublishStorageDealsParams,
    PublishStorageDealsReturn,
};
use fil_actor_miner::{
    aggregate_pre_commit_network_fee, max_prove_commit_duration,
    new_deadline_info_from_offset_and_epoch, DeadlineInfo, Method as MinerMethod, PoStPartition,
    PowerPair, PreCommitSectorBatchParams, SectorPreCommitInfo, SectorPreCommitOnChainInfo,
    State as MinerState, SubmitWindowedPoStParams,
};
use fil_actor_multisig::Method as MultisigMethod;
use fil_actor_multisig::ProposeParams;
use fil_actor_power::{
    CreateMinerParams, CreateMinerReturn, Method as PowerMethod, UpdateClaimedPowerParams,
};
use fil_actor_reward::Method as RewardMethod;
use fil_actor_verifreg::{Method as VerifregMethod, VerifierParams};
use fvm_ipld_bitfield::{BitField, UnvalidatedBitField};
use fvm_ipld_encoding::{BytesDe, Cbor, RawBytes};
use fvm_shared::address::{Address, BLS_PUB_LEN};
use fvm_shared::crypto::signature::{Signature, SignatureType};
use fvm_shared::econ::TokenAmount;
use fvm_shared::error::ExitCode;
use fvm_shared::piece::PaddedPieceSize;
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
    let mid = v.normalize_address(&maddr).unwrap();
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
            to: mid,
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
    let mstate = v.get_state::<MinerState>(mid).unwrap();
    (0..count)
        .map(|i| mstate.get_precommitted_sector(v.store, sector_number_base + i).unwrap().unwrap())
        .collect()
}

pub fn advance_by_deadline_to_epoch(v: VM, maddr: Address, e: ChainEpoch) -> (VM, DeadlineInfo) {
    // keep advancing until the epoch of interest is within the deadline
    // if e is dline.last() == dline.close -1 cron is not run
    let (v, dline_info) = advance_by_deadline(v, maddr, |dline_info| dline_info.close < e);
    (v.with_epoch(e), dline_info)
}

pub fn advance_by_deadline_to_index(v: VM, maddr: Address, i: u64) -> (VM, DeadlineInfo) {
    advance_by_deadline(v, maddr, |dline_info| dline_info.index != i)
}

pub fn advance_by_deadline_to_epoch_while_proving(
    mut v: VM,
    maddr: Address,
    worker: Address,
    s: SectorNumber,
    e: ChainEpoch,
) -> VM {
    let mut dline_info;
    let (d, p_idx) = sector_deadline(&v, maddr, s);
    loop {
        // stop if either we reach deadline of e or the proving deadline for sector s
        (v, dline_info) = advance_by_deadline(v, maddr, |dline_info| {
            dline_info.index != d && dline_info.close < e
        });
        if dline_info.close > e {
            // in the case e is within the proving deadline don't post, leave that to the caller
            return v.with_epoch(e);
        }
        submit_windowed_post(&v, worker, maddr, dline_info, p_idx, PowerPair::zero());
        v = advance_by_deadline_to_index(
            v,
            maddr,
            d + 1 % &Policy::default().wpost_period_deadlines,
        )
        .0
    }
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
    partition_idx: u64,
    new_power: PowerPair,
) {
    let params = SubmitWindowedPoStParams {
        deadline: dline_info.index,
        partitions: vec![PoStPartition {
            index: partition_idx,
            skipped: fvm_ipld_bitfield::UnvalidatedBitField::Validated(BitField::new()),
        }],
        proofs: vec![PoStProof {
            post_proof: RegisteredPoStProof::StackedDRGWindow32GiBV1,
            proof_bytes: vec![],
        }],
        chain_commit_epoch: dline_info.challenge,
        chain_commit_rand: Randomness(TEST_VM_RAND_STRING.to_owned().into_bytes()),
    };
    apply_ok(v, worker, maddr, TokenAmount::zero(), MinerMethod::SubmitWindowedPoSt as u64, params);
    let mut subinvocs = None;
    if new_power != PowerPair::zero() {
        let update_power_params = serialize(
            &UpdateClaimedPowerParams {
                raw_byte_delta: new_power.raw,
                quality_adjusted_delta: new_power.qa,
            },
            "update claim params",
        )
        .unwrap();
        subinvocs = Some(vec![ExpectInvocation {
            to: *STORAGE_POWER_ACTOR_ADDR,
            method: PowerMethod::UpdateClaimedPower as u64,
            params: Some(update_power_params),
            ..Default::default()
        }]);
    }

    ExpectInvocation {
        to: maddr,
        method: MinerMethod::SubmitWindowedPoSt as u64,
        subinvocs,
        ..Default::default()
    }
    .matches(v.take_invocations().last().unwrap());
}

pub fn add_verifier(v: &VM, verifier: Address, data_cap: StoragePower) {
    let add_verifier_params = VerifierParams { address: verifier, allowance: data_cap };
    // root address is msig, send proposal from root key
    let proposal = ProposeParams {
        to: *VERIFIED_REGISTRY_ACTOR_ADDR,
        value: TokenAmount::zero(),
        method: VerifregMethod::AddVerifier as u64,
        params: serialize(&add_verifier_params, "verifreg add verifier params").unwrap(),
    };

    apply_ok(
        v,
        TEST_VERIFREG_ROOT_SIGNER_ADDR,
        TEST_VERIFREG_ROOT_ADDR,
        TokenAmount::zero(),
        MultisigMethod::Propose as u64,
        proposal,
    );
}

pub fn publish_deal(
    v: &VM,
    provider: Address,
    deal_client: Address,
    miner_id: Address,
    deal_label: String,
    piece_size: PaddedPieceSize,
    verified_deal: bool,
    deal_start: ChainEpoch,
    deal_lifetime: ChainEpoch,
) -> PublishStorageDealsReturn {
    let label = Label::String(deal_label.to_string());
    let deal = DealProposal {
        piece_cid: make_piece_cid(deal_label.as_bytes()),
        piece_size,
        verified_deal,
        client: deal_client,
        provider: miner_id,
        label,
        start_epoch: deal_start,
        end_epoch: deal_start + deal_lifetime,
        storage_price_per_epoch: TokenAmount::from((1 << 20) as u64),
        provider_collateral: TokenAmount::from(2e18 as u64),
        client_collateral: TokenAmount::from(1e18 as u64),
    };

    let publish_params = PublishStorageDealsParams {
        deals: vec![ClientDealProposal {
            proposal: deal,
            client_signature: Signature { sig_type: SignatureType::BLS, bytes: vec![] },
        }],
    };
    let ret: PublishStorageDealsReturn = apply_ok(
        &v,
        provider,
        *STORAGE_MARKET_ACTOR_ADDR,
        TokenAmount::zero(),
        MarketMethod::PublishStorageDeals as u64,
        publish_params,
    )
    .deserialize()
    .unwrap();

    let mut expect_publish_invocs = vec![
        ExpectInvocation {
            to: miner_id,
            method: MinerMethod::ControlAddresses as u64,
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
    ];
    if verified_deal {
        expect_publish_invocs.push(ExpectInvocation {
            to: *VERIFIED_REGISTRY_ACTOR_ADDR,
            method: VerifregMethod::UseBytes as u64,
            ..Default::default()
        })
    }
    ExpectInvocation {
        to: *STORAGE_MARKET_ACTOR_ADDR,
        method: MarketMethod::PublishStorageDeals as u64,
        subinvocs: Some(expect_publish_invocs),
        ..Default::default()
    }
    .matches(v.take_invocations().last().unwrap());

    ret
}

pub fn make_bitfield(bits: &[u64]) -> UnvalidatedBitField {
    UnvalidatedBitField::Validated(BitField::try_from_bits(bits.iter().copied()).unwrap())
}
