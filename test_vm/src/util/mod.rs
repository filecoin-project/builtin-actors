use std::cmp::min;

use frc46_token::receiver::{FRC46TokenReceived, FRC46_TOKEN_TYPE};
use frc46_token::token::types::{TransferFromParams, TransferParams};
use fvm_actor_utils::receiver::UniversalReceiverParams;
use fvm_ipld_bitfield::BitField;
use fvm_ipld_encoding::{BytesDe, RawBytes};
use fvm_shared::address::{Address, BLS_PUB_LEN};
use fvm_shared::crypto::signature::{Signature, SignatureType};
use fvm_shared::deal::DealID;
use fvm_shared::econ::TokenAmount;
use fvm_shared::error::ExitCode;
use fvm_shared::piece::PaddedPieceSize;
use fvm_shared::sector::{PoStProof, RegisteredPoStProof, RegisteredSealProof, SectorNumber};
use fvm_shared::{MethodNum, METHOD_SEND};
use rand::prelude::*;
use rand_chacha::ChaCha8Rng;
use serde::Serialize;

use fil_actor_cron::Method as CronMethod;
use fil_actor_datacap::Method as DataCapMethod;
use fil_actor_market::ext::verifreg::{
    AllocationRequest, AllocationRequests, ClaimExtensionRequest,
};
use fil_actor_market::{
    ClientDealProposal, DealProposal, Label, Method as MarketMethod, PublishStorageDealsParams,
    PublishStorageDealsReturn, SectorDeals, MARKET_NOTIFY_DEAL_METHOD,
};
use fil_actor_miner::{
    aggregate_pre_commit_network_fee, aggregate_prove_commit_network_fee,
    max_prove_commit_duration, new_deadline_info_from_offset_and_epoch, ChangeBeneficiaryParams,
    CompactCommD, Deadline, DeadlineInfo, DeclareFaultsRecoveredParams, ExpirationExtension2,
    ExtendSectorExpiration2Params, GetBeneficiaryReturn, Method as MinerMethod, PoStPartition,
    PowerPair, PreCommitSectorBatchParams, PreCommitSectorBatchParams2, PreCommitSectorParams,
    ProveCommitAggregateParams, ProveCommitSectorParams, RecoveryDeclaration, SectorClaim,
    SectorOnChainInfo, SectorPreCommitInfo, SectorPreCommitOnChainInfo, State as MinerState,
    SubmitWindowedPoStParams, WithdrawBalanceParams, WithdrawBalanceReturn,
};
use fil_actor_multisig::Method as MultisigMethod;
use fil_actor_multisig::ProposeParams;
use fil_actor_power::{CreateMinerParams, CreateMinerReturn, Method as PowerMethod};
use fil_actor_verifreg::ext::datacap::MintParams;
use fil_actor_verifreg::{
    AddVerifiedClientParams, AllocationID, ClaimID, ClaimTerm, ExtendClaimTermsParams,
    Method as VerifregMethod, RemoveExpiredAllocationsParams, VerifierParams,
};
use fil_actors_runtime::cbor::deserialize;
use fil_actors_runtime::runtime::policy_constants::{
    MARKET_DEFAULT_ALLOCATION_TERM_BUFFER, MAXIMUM_VERIFIED_ALLOCATION_EXPIRATION,
};
use fil_actors_runtime::{DATACAP_TOKEN_ACTOR_ID, VERIFIED_REGISTRY_ACTOR_ID};
use fil_builtin_actors_state::check::check_state_invariants;

use crate::expects::Expect;
use crate::trace::ExpectInvocation;
use crate::*;

mod blockstore;
pub use blockstore::DynBlockstore;

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

pub fn create_accounts(v: &dyn VM, count: u64, balance: &TokenAmount) -> Vec<Address> {
    create_accounts_seeded(v, count, balance, ACCOUNT_SEED)
}

pub fn create_accounts_seeded(
    v: &dyn VM,
    count: u64,
    balance: &TokenAmount,
    seed: u64,
) -> Vec<Address> {
    let pk_addrs = pk_addrs_from(seed, count);
    // Send funds from faucet to pk address, creating account actor
    for pk_addr in pk_addrs.clone() {
        apply_ok(v, &TEST_FAUCET_ADDR, &pk_addr, balance, METHOD_SEND, None::<RawBytes>);
    }
    // Normalize pk address to return id address of account actor
    pk_addrs.iter().map(|pk_addr| v.resolve_id_address(pk_addr).unwrap()).collect()
}

pub fn apply_ok<S: Serialize>(
    v: &dyn VM,
    from: &Address,
    to: &Address,
    value: &TokenAmount,
    method: MethodNum,
    params: Option<S>,
) -> RawBytes {
    apply_code(v, from, to, value, method, params, ExitCode::OK)
}

pub fn apply_code<S: Serialize>(
    v: &dyn VM,
    from: &Address,
    to: &Address,
    value: &TokenAmount,
    method: MethodNum,
    params: Option<S>,
    code: ExitCode,
) -> RawBytes {
    let params = params.map(|p| IpldBlock::serialize_cbor(&p).unwrap().unwrap());
    let res = v.execute_message(from, to, value, method, params).unwrap();
    assert_eq!(code, res.code, "expected code {}, got {} ({})", code, res.code, res.message);
    res.ret.map_or(RawBytes::default(), |b| RawBytes::new(b.data))
}

/// Convenience function to create an IpldBlock from a serializable object
pub fn serialize_ok<S: Serialize>(s: &S) -> IpldBlock {
    IpldBlock::serialize_cbor(s).unwrap().unwrap()
}

pub fn cron_tick(v: &dyn VM) {
    apply_ok(
        v,
        &SYSTEM_ACTOR_ADDR,
        &CRON_ACTOR_ADDR,
        &TokenAmount::zero(),
        CronMethod::EpochTick as u64,
        None::<RawBytes>,
    );
}

pub fn create_miner(
    v: &dyn VM,
    owner: &Address,
    worker: &Address,
    post_proof_type: RegisteredPoStProof,
    balance: &TokenAmount,
) -> (Address, Address) {
    let multiaddrs = vec![BytesDe("multiaddr".as_bytes().to_vec())];
    let peer_id = "miner".as_bytes().to_vec();
    let params = CreateMinerParams {
        owner: *owner,
        worker: *worker,
        window_post_proof_type: post_proof_type,
        peer: peer_id,
        multiaddrs,
    };

    let params = IpldBlock::serialize_cbor(&params).unwrap().unwrap();
    let res: CreateMinerReturn = v
        .execute_message(
            owner,
            &STORAGE_POWER_ACTOR_ADDR,
            balance,
            PowerMethod::CreateMiner as u64,
            Some(params),
        )
        .unwrap()
        .ret
        .unwrap()
        .deserialize()
        .unwrap();
    (res.id_address, res.robust_address)
}

pub fn miner_precommit_sector(
    v: &dyn VM,
    worker: &Address,
    miner_id: &Address,
    seal_proof: RegisteredSealProof,
    sector_number: SectorNumber,
    deal_ids: Vec<DealID>,
    expiration: ChainEpoch,
) -> SectorPreCommitOnChainInfo {
    let sealed_cid = make_sealed_cid(b"s100");

    let params = PreCommitSectorParams {
        seal_proof,
        sector_number,
        sealed_cid,
        seal_rand_epoch: v.epoch() - 1,
        deal_ids,
        expiration,
        replace_capacity: false,
        replace_sector_deadline: 0,
        replace_sector_partition: 0,
        replace_sector_number: 0,
    };

    apply_ok(
        v,
        worker,
        miner_id,
        &TokenAmount::zero(),
        MinerMethod::PreCommitSector as u64,
        Some(params),
    );

    let state: MinerState = get_state(v, miner_id).unwrap();
    state
        .get_precommitted_sector(&DynBlockstore::wrap(v.blockstore()), sector_number)
        .unwrap()
        .unwrap()
}

pub fn miner_prove_sector(
    v: &dyn VM,
    worker: &Address,
    miner_id: &Address,
    sector_number: SectorNumber,
) {
    let prove_commit_params = ProveCommitSectorParams { sector_number, proof: vec![] };
    apply_ok(
        v,
        worker,
        miner_id,
        &TokenAmount::zero(),
        MinerMethod::ProveCommitSector as u64,
        Some(prove_commit_params),
    );

    ExpectInvocation {
        from: *worker,
        to: *miner_id,
        method: MinerMethod::ProveCommitSector as u64,
        subinvocs: Some(vec![ExpectInvocation {
            from: *miner_id,
            to: STORAGE_POWER_ACTOR_ADDR,
            method: PowerMethod::SubmitPoRepForBulkVerify as u64,
            ..Default::default()
        }]),
        ..Default::default()
    }
    .matches(v.take_invocations().last().unwrap());
}

pub struct PrecommitMetadata {
    pub deals: Vec<DealID>,
    pub commd: CompactCommD,
}

#[allow(clippy::too_many_arguments)]
pub fn precommit_sectors_v2(
    v: &dyn VM,
    count: usize,
    batch_size: usize,
    metadata: Vec<PrecommitMetadata>, // Per-sector deal metadata, or empty vector for no deals.
    worker: &Address,
    maddr: &Address,
    seal_proof: RegisteredSealProof,
    sector_number_base: SectorNumber,
    expect_cron_enroll: bool,
    exp: Option<ChainEpoch>,
    v2: bool,
) -> Vec<SectorPreCommitOnChainInfo> {
    let mid = v.resolve_id_address(maddr).unwrap();
    let expiration = match exp {
        None => {
            v.epoch()
                + Policy::default().min_sector_expiration
                + max_prove_commit_duration(&Policy::default(), seal_proof).unwrap()
        }
        Some(e) => e,
    };

    let mut sector_idx: usize = 0;
    let no_deals = PrecommitMetadata { deals: vec![], commd: CompactCommD::default() };
    let mut sectors_with_deals: Vec<SectorDeals> = vec![];
    while sector_idx < count {
        let msg_sector_idx_base = sector_idx;
        let mut invocs = vec![Expect::reward_this_epoch(mid), Expect::power_current_total(mid)];
        if !v2 {
            let mut param_sectors = Vec::<PreCommitSectorParams>::new();
            let mut j = 0;
            while j < batch_size && sector_idx < count {
                let sector_number = sector_number_base + sector_idx as u64;
                let sector_meta = metadata.get(sector_idx).unwrap_or(&no_deals);
                param_sectors.push(PreCommitSectorParams {
                    seal_proof,
                    sector_number,
                    sealed_cid: make_sealed_cid(format!("sn: {}", sector_number).as_bytes()),
                    seal_rand_epoch: v.epoch() - 1,
                    deal_ids: sector_meta.deals.clone().clone(),
                    expiration,
                    ..Default::default()
                });
                if !sector_meta.deals.is_empty() {
                    sectors_with_deals.push(SectorDeals {
                        sector_type: seal_proof,
                        sector_expiry: expiration,
                        deal_ids: sector_meta.deals.clone(),
                    });
                }
                sector_idx += 1;
                j += 1;
            }
            if !sectors_with_deals.is_empty() {
                invocs.push(Expect::market_verify_deals(mid, sectors_with_deals.clone()));
            }
            if param_sectors.len() > 1 {
                invocs.push(Expect::burn(
                    mid,
                    Some(aggregate_pre_commit_network_fee(
                        param_sectors.len() as i64,
                        &TokenAmount::zero(),
                    )),
                ));
            }
            if expect_cron_enroll && msg_sector_idx_base == 0 {
                invocs.push(Expect::power_enrol_cron(mid));
            }

            apply_ok(
                v,
                worker,
                maddr,
                &TokenAmount::zero(),
                MinerMethod::PreCommitSectorBatch as u64,
                Some(PreCommitSectorBatchParams { sectors: param_sectors.clone() }),
            );
            let expect = ExpectInvocation {
                from: *worker,
                to: mid,
                method: MinerMethod::PreCommitSectorBatch as u64,
                params: Some(
                    IpldBlock::serialize_cbor(&PreCommitSectorBatchParams {
                        sectors: param_sectors,
                    })
                    .unwrap(),
                ),
                subinvocs: Some(invocs),
                ..Default::default()
            };
            expect.matches(v.take_invocations().last().unwrap())
        } else {
            let mut param_sectors = Vec::<SectorPreCommitInfo>::new();
            let mut j = 0;
            while j < batch_size && sector_idx < count {
                let sector_number = sector_number_base + sector_idx as u64;
                let sector_meta = metadata.get(sector_idx).unwrap_or(&no_deals);
                param_sectors.push(SectorPreCommitInfo {
                    seal_proof,
                    sector_number,
                    sealed_cid: make_sealed_cid(format!("sn: {}", sector_number).as_bytes()),
                    seal_rand_epoch: v.epoch() - 1,
                    deal_ids: sector_meta.deals.clone(),
                    expiration,
                    unsealed_cid: sector_meta.commd.clone(),
                });
                if !sector_meta.deals.is_empty() {
                    sectors_with_deals.push(SectorDeals {
                        sector_type: seal_proof,
                        sector_expiry: expiration,
                        deal_ids: sector_meta.deals.clone(),
                    });
                }
                sector_idx += 1;
                j += 1;
            }
            if !sectors_with_deals.is_empty() {
                invocs.push(Expect::market_verify_deals(mid, sectors_with_deals.clone()));
            }
            if param_sectors.len() > 1 {
                invocs.push(Expect::burn(
                    mid,
                    Some(aggregate_pre_commit_network_fee(
                        param_sectors.len() as i64,
                        &TokenAmount::zero(),
                    )),
                ));
            }
            if expect_cron_enroll && msg_sector_idx_base == 0 {
                invocs.push(Expect::power_enrol_cron(mid));
            }

            apply_ok(
                v,
                worker,
                maddr,
                &TokenAmount::zero(),
                MinerMethod::PreCommitSectorBatch2 as u64,
                Some(PreCommitSectorBatchParams2 { sectors: param_sectors.clone() }),
            );

            let expect = ExpectInvocation {
                from: *worker,
                to: mid,
                method: MinerMethod::PreCommitSectorBatch2 as u64,
                params: Some(
                    IpldBlock::serialize_cbor(&PreCommitSectorBatchParams2 {
                        sectors: param_sectors,
                    })
                    .unwrap(),
                ),
                subinvocs: Some(invocs),
                ..Default::default()
            };
            expect.matches(v.take_invocations().last().unwrap())
        }
    }
    // extract chain state
    let mstate: MinerState = get_state(v, &mid).unwrap();
    (0..count)
        .map(|i| {
            mstate
                .get_precommitted_sector(
                    &DynBlockstore::wrap(v.blockstore()),
                    sector_number_base + i as u64,
                )
                .unwrap()
                .unwrap()
        })
        .collect()
}

#[allow(clippy::too_many_arguments)]
pub fn precommit_sectors(
    v: &dyn VM,
    count: usize,
    batch_size: usize,
    worker: &Address,
    maddr: &Address,
    seal_proof: RegisteredSealProof,
    sector_number_base: SectorNumber,
    expect_cron_enroll: bool,
    exp: Option<ChainEpoch>,
) -> Vec<SectorPreCommitOnChainInfo> {
    precommit_sectors_v2(
        v,
        count,
        batch_size,
        vec![], // no deals
        worker,
        maddr,
        seal_proof,
        sector_number_base,
        expect_cron_enroll,
        exp,
        false,
    )
}

pub fn prove_commit_sectors(
    v: &dyn VM,
    worker: &Address,
    maddr: &Address,
    precommits: Vec<SectorPreCommitOnChainInfo>,
    aggregate_size: usize,
) {
    let mut precommit_infos = precommits.as_slice();
    while !precommit_infos.is_empty() {
        let batch_size = min(aggregate_size, precommit_infos.len());
        let to_prove = &precommit_infos[0..batch_size];
        precommit_infos = &precommit_infos[batch_size..];
        let b: Vec<u64> = to_prove.iter().map(|p| p.info.sector_number).collect();

        let prove_commit_aggregate_params = ProveCommitAggregateParams {
            sector_numbers: make_bitfield(b.as_slice()),
            aggregate_proof: vec![],
        };

        let prove_commit_aggregate_params_ser =
            IpldBlock::serialize_cbor(&prove_commit_aggregate_params).unwrap();

        apply_ok(
            v,
            worker,
            maddr,
            &TokenAmount::zero(),
            MinerMethod::ProveCommitAggregate as u64,
            Some(prove_commit_aggregate_params),
        );

        let expected_fee =
            aggregate_prove_commit_network_fee(to_prove.len() as i64, &TokenAmount::zero());
        ExpectInvocation {
            from: *worker,
            to: *maddr,
            method: MinerMethod::ProveCommitAggregate as u64,
            params: Some(prove_commit_aggregate_params_ser),
            subinvocs: Some(vec![
                Expect::reward_this_epoch(*maddr),
                Expect::power_current_total(*maddr),
                Expect::power_update_pledge(*maddr, None),
                Expect::burn(*maddr, Some(expected_fee)),
            ]),
            ..Default::default()
        }
        .matches(v.take_invocations().last().unwrap());
    }
}

#[allow(clippy::too_many_arguments)]
pub fn miner_extend_sector_expiration2(
    v: &dyn VM,
    worker: &Address,
    miner: &Address,
    deadline: u64,
    partition: u64,
    sectors_without_claims: Vec<u64>,
    sectors_with_claims: Vec<SectorClaim>,
    new_expiration: ChainEpoch,
    power_delta: PowerPair,
) {
    let extension_params = ExtendSectorExpiration2Params {
        extensions: vec![ExpirationExtension2 {
            deadline,
            partition,
            sectors: BitField::try_from_bits(sectors_without_claims.iter().copied()).unwrap(),
            sectors_with_claims: sectors_with_claims.clone(),
            new_expiration,
        }],
    };

    apply_ok(
        v,
        worker,
        miner,
        &TokenAmount::zero(),
        MinerMethod::ExtendSectorExpiration2 as u64,
        Some(extension_params),
    );

    let mut claim_ids = vec![];
    for sector_claim in sectors_with_claims {
        claim_ids = sector_claim.maintain_claims.clone();
        claim_ids.extend(sector_claim.drop_claims);
    }

    let mut subinvocs = vec![];
    if !claim_ids.is_empty() {
        subinvocs.push(Expect::verifreg_get_claims(*miner, miner.id().unwrap(), claim_ids))
    }
    subinvocs.push(Expect::reward_this_epoch(*miner));
    subinvocs.push(Expect::power_current_total(*miner));
    if !power_delta.is_zero() {
        subinvocs.push(Expect::power_update_claim(*miner, power_delta));
    }

    ExpectInvocation {
        from: *worker,
        to: *miner,
        method: MinerMethod::ExtendSectorExpiration2 as u64,
        subinvocs: Some(subinvocs),
        ..Default::default()
    }
    .matches(v.take_invocations().last().unwrap());
}

pub fn advance_by_deadline_to_epoch(v: &dyn VM, maddr: &Address, e: ChainEpoch) -> DeadlineInfo {
    // keep advancing until the epoch of interest is within the deadline
    // if e is dline.last() == dline.close -1 cron is not run
    let dline_info = advance_by_deadline(v, maddr, |dline_info| dline_info.close < e);
    v.set_epoch(e);
    dline_info
}

pub fn advance_by_deadline_to_index(v: &dyn VM, maddr: &Address, i: u64) -> DeadlineInfo {
    advance_by_deadline(v, maddr, |dline_info| dline_info.index != i)
}

pub fn advance_by_deadline_to_epoch_while_proving(
    v: &dyn VM,
    maddr: &Address,
    worker: &Address,
    s: SectorNumber,
    e: ChainEpoch,
) {
    let mut dline_info;
    let (d, p_idx) = sector_deadline(v, maddr, s);
    loop {
        // stop if either we reach deadline of e or the proving deadline for sector s
        dline_info = advance_by_deadline(v, maddr, |dline_info| {
            dline_info.index != d && dline_info.close < e
        });
        if dline_info.close > e {
            // in the case e is within the proving deadline don't post, leave that to the caller
            v.set_epoch(e);
            return;
        }
        submit_windowed_post(v, worker, maddr, dline_info, p_idx, None);
        advance_by_deadline_to_index(v, maddr, d + 1 % &Policy::default().wpost_period_deadlines);
    }
}

pub fn advance_to_proving_deadline(
    v: &dyn VM,
    maddr: &Address,
    s: SectorNumber,
) -> (DeadlineInfo, u64) {
    let (d, p) = sector_deadline(v, maddr, s);
    let dline_info = advance_by_deadline_to_index(v, maddr, d);
    v.set_epoch(dline_info.open);
    (dline_info, p)
}

fn advance_by_deadline<F>(v: &dyn VM, maddr: &Address, more: F) -> DeadlineInfo
where
    F: Fn(DeadlineInfo) -> bool,
{
    loop {
        let dline_info = miner_dline_info(v, maddr);
        if !more(dline_info) {
            return dline_info;
        }
        v.set_epoch(dline_info.last());
        cron_tick(v);
        let next = v.epoch() + 1;
        v.set_epoch(next);
    }
}

pub fn get_state<T: DeserializeOwned>(v: &dyn VM, a: &Address) -> Option<T> {
    let cid = v.actor_root(a).unwrap();
    v.blockstore().get(&cid).unwrap().map(|slice| fvm_ipld_encoding::from_slice(&slice).unwrap())
}

pub fn miner_balance(v: &dyn VM, m: &Address) -> MinerBalances {
    let st: MinerState = get_state(v, m).unwrap();
    MinerBalances {
        available_balance: st.get_available_balance(&v.balance(m)).unwrap(),
        vesting_balance: st.locked_funds,
        initial_pledge: st.initial_pledge,
        pre_commit_deposit: st.pre_commit_deposits,
    }
}

pub fn miner_info(v: &dyn VM, m: &Address) -> MinerInfo {
    let st: MinerState = get_state(v, m).unwrap();
    DynBlockstore::wrap(v.blockstore()).get_cbor(&st.info).unwrap().unwrap()
}

pub fn miner_dline_info(v: &dyn VM, m: &Address) -> DeadlineInfo {
    let st: MinerState = get_state(v, m).unwrap();
    new_deadline_info_from_offset_and_epoch(&Policy::default(), st.proving_period_start, v.epoch())
}

pub fn sector_deadline(v: &dyn VM, m: &Address, s: SectorNumber) -> (u64, u64) {
    let st: MinerState = get_state(v, m).unwrap();
    st.find_sector(&Policy::default(), &DynBlockstore::wrap(v.blockstore()), s).unwrap()
}

pub fn check_sector_active(v: &dyn VM, m: &Address, s: SectorNumber) -> bool {
    let (d_idx, p_idx) = sector_deadline(v, m, s);
    let st: MinerState = get_state(v, m).unwrap();
    st.check_sector_active(
        &Policy::default(),
        &DynBlockstore::wrap(v.blockstore()),
        d_idx,
        p_idx,
        s,
        true,
    )
    .unwrap()
}

pub fn check_sector_faulty(
    v: &dyn VM,
    m: &Address,
    d_idx: u64,
    p_idx: u64,
    s: SectorNumber,
) -> bool {
    let st: MinerState = get_state(v, m).unwrap();
    let bs = &DynBlockstore::wrap(v.blockstore());
    let deadlines = st.load_deadlines(bs).unwrap();
    let deadline = deadlines.load_deadline(&Policy::default(), bs, d_idx).unwrap();
    let partition = deadline.load_partition(bs, p_idx).unwrap();
    partition.faults.get(s)
}

pub fn deadline_state(v: &dyn VM, m: &Address, d_idx: u64) -> Deadline {
    let st: MinerState = get_state(v, m).unwrap();
    let bs = &DynBlockstore::wrap(v.blockstore());
    let deadlines = st.load_deadlines(bs).unwrap();
    deadlines.load_deadline(&Policy::default(), bs, d_idx).unwrap()
}

pub fn sector_info(v: &dyn VM, m: &Address, s: SectorNumber) -> SectorOnChainInfo {
    let st: MinerState = get_state(v, m).unwrap();
    st.get_sector(&DynBlockstore::wrap(v.blockstore()), s).unwrap().unwrap()
}

pub fn miner_power(v: &dyn VM, m: &Address) -> PowerPair {
    let st: PowerState = get_state(v, &STORAGE_POWER_ACTOR_ADDR).unwrap();
    let claim = st.get_claim(&DynBlockstore::wrap(v.blockstore()), m).unwrap().unwrap();
    PowerPair::new(claim.raw_byte_power, claim.quality_adj_power)
}

pub fn declare_recovery(
    v: &dyn VM,
    worker: &Address,
    maddr: &Address,
    deadline: u64,
    partition: u64,
    sector_number: SectorNumber,
) {
    let recover_params = DeclareFaultsRecoveredParams {
        recoveries: vec![RecoveryDeclaration {
            deadline,
            partition,
            sectors: BitField::try_from_bits([sector_number].iter().copied()).unwrap(),
        }],
    };

    apply_ok(
        v,
        worker,
        maddr,
        &TokenAmount::zero(),
        MinerMethod::DeclareFaultsRecovered as u64,
        Some(recover_params),
    );
}

pub fn submit_windowed_post(
    v: &dyn VM,
    worker: &Address,
    maddr: &Address,
    dline_info: DeadlineInfo,
    partition_idx: u64,
    new_power: Option<PowerPair>,
) {
    let params = SubmitWindowedPoStParams {
        deadline: dline_info.index,
        partitions: vec![PoStPartition { index: partition_idx, skipped: BitField::new() }],
        proofs: vec![PoStProof {
            post_proof: RegisteredPoStProof::StackedDRGWindow32GiBV1P1,
            proof_bytes: vec![],
        }],
        chain_commit_epoch: dline_info.challenge,
        chain_commit_rand: Randomness(TEST_VM_RAND_ARRAY.into()),
    };
    apply_ok(
        v,
        worker,
        maddr,
        &TokenAmount::zero(),
        MinerMethod::SubmitWindowedPoSt as u64,
        Some(params),
    );
    let mut subinvocs = None; // Unchecked unless provided
    if let Some(new_pow) = new_power {
        if new_pow == PowerPair::zero() {
            subinvocs = Some(vec![])
        } else {
            subinvocs = Some(vec![Expect::power_update_claim(*maddr, new_pow)])
        }
    }

    ExpectInvocation {
        from: *worker,
        to: *maddr,
        method: MinerMethod::SubmitWindowedPoSt as u64,
        subinvocs,
        ..Default::default()
    }
    .matches(v.take_invocations().last().unwrap());
}

pub fn change_beneficiary(
    v: &dyn VM,
    from: &Address,
    maddr: &Address,
    beneficiary_change_proposal: &ChangeBeneficiaryParams,
) {
    apply_ok(
        v,
        from,
        maddr,
        &TokenAmount::zero(),
        MinerMethod::ChangeBeneficiary as u64,
        Some(beneficiary_change_proposal.clone()),
    );
}

pub fn check_invariants(vm: &dyn VM) -> anyhow::Result<MessageAccumulator> {
    check_state_invariants(
        &vm.actor_manifest(),
        &vm.policy(),
        Tree::load(&DynBlockstore::wrap(vm.blockstore()), &vm.state_root()).unwrap(),
        &vm.total_fil(),
        vm.epoch() - 1,
    )
}

pub fn assert_invariants(vm: &dyn VM) {
    check_invariants(vm).unwrap().assert_empty()
}

pub fn expect_invariants(vm: &dyn VM, expected_patterns: &[Regex]) {
    check_invariants(vm).unwrap().assert_expected(expected_patterns)
}

pub fn get_network_stats(vm: &dyn VM) -> NetworkStats {
    let power_state: PowerState = get_state(vm, &STORAGE_POWER_ACTOR_ADDR).unwrap();
    let reward_state: RewardState = get_state(vm, &REWARD_ACTOR_ADDR).unwrap();
    let market_state: MarketState = get_state(vm, &STORAGE_MARKET_ACTOR_ADDR).unwrap();

    NetworkStats {
        total_raw_byte_power: power_state.total_raw_byte_power,
        total_bytes_committed: power_state.total_bytes_committed,
        total_quality_adj_power: power_state.total_quality_adj_power,
        total_qa_bytes_committed: power_state.total_qa_bytes_committed,
        total_pledge_collateral: power_state.total_pledge_collateral,
        this_epoch_raw_byte_power: power_state.this_epoch_raw_byte_power,
        this_epoch_quality_adj_power: power_state.this_epoch_quality_adj_power,
        this_epoch_pledge_collateral: power_state.this_epoch_pledge_collateral,
        miner_count: power_state.miner_count,
        miner_above_min_power_count: power_state.miner_above_min_power_count,
        this_epoch_reward: reward_state.this_epoch_reward,
        this_epoch_reward_smoothed: reward_state.this_epoch_reward_smoothed,
        this_epoch_baseline_power: reward_state.this_epoch_baseline_power,
        total_storage_power_reward: reward_state.total_storage_power_reward,
        total_client_locked_collateral: market_state.total_client_locked_collateral,
        total_provider_locked_collateral: market_state.total_provider_locked_collateral,
        total_client_storage_fee: market_state.total_client_storage_fee,
    }
}

pub fn get_beneficiary(v: &dyn VM, from: &Address, m_addr: &Address) -> GetBeneficiaryReturn {
    apply_ok(
        v,
        from,
        m_addr,
        &TokenAmount::zero(),
        MinerMethod::GetBeneficiary as u64,
        None::<RawBytes>,
    )
    .deserialize()
    .unwrap()
}

pub fn change_owner_address(
    v: &dyn VM,
    from: &Address,
    m_addr: &Address,
    new_miner_addr: &Address,
) {
    apply_ok(
        v,
        from,
        m_addr,
        &TokenAmount::zero(),
        MinerMethod::ChangeOwnerAddress as u64,
        Some(new_miner_addr),
    );
}

pub fn withdraw_balance(
    v: &dyn VM,
    from: &Address,
    m_addr: &Address,
    to_withdraw_amount: &TokenAmount,
    expect_withdraw_amount: &TokenAmount,
) {
    let params = WithdrawBalanceParams { amount_requested: to_withdraw_amount.clone() };
    let withdraw_return: WithdrawBalanceReturn = apply_ok(
        v,
        from,
        m_addr,
        &TokenAmount::zero(),
        MinerMethod::WithdrawBalance as u64,
        Some(params.clone()),
    )
    .deserialize()
    .unwrap();

    if expect_withdraw_amount.is_positive() {
        let withdraw_balance_params_se = IpldBlock::serialize_cbor(&params).unwrap();
        ExpectInvocation {
            from: *from,
            to: *m_addr,
            method: MinerMethod::WithdrawBalance as u64,
            params: Some(withdraw_balance_params_se),
            subinvocs: Some(vec![Expect::send(
                *m_addr,
                *from,
                Some(expect_withdraw_amount.clone()),
            )]),
            ..Default::default()
        }
        .matches(v.take_invocations().last().unwrap());
    }
    assert_eq!(expect_withdraw_amount, &withdraw_return.amount_withdrawn);
}

pub fn submit_invalid_post(
    v: &dyn VM,
    worker: &Address,
    maddr: &Address,
    dline_info: DeadlineInfo,
    partition_idx: u64,
) {
    let params = SubmitWindowedPoStParams {
        deadline: dline_info.index,
        partitions: vec![PoStPartition { index: partition_idx, skipped: BitField::new() }],
        proofs: vec![PoStProof {
            post_proof: RegisteredPoStProof::StackedDRGWindow32GiBV1P1,
            proof_bytes: TEST_VM_INVALID_POST.as_bytes().to_vec(),
        }],
        chain_commit_epoch: dline_info.challenge,
        chain_commit_rand: Randomness(TEST_VM_RAND_ARRAY.into()),
    };
    apply_ok(
        v,
        worker,
        maddr,
        &TokenAmount::zero(),
        MinerMethod::SubmitWindowedPoSt as u64,
        Some(params),
    );
}

pub fn verifreg_add_verifier(v: &dyn VM, verifier: &Address, data_cap: StoragePower) {
    let add_verifier_params = VerifierParams { address: *verifier, allowance: data_cap };
    // root address is msig, send proposal from root key
    let proposal = ProposeParams {
        to: VERIFIED_REGISTRY_ACTOR_ADDR,
        value: TokenAmount::zero(),
        method: VerifregMethod::AddVerifier as u64,
        params: serialize(&add_verifier_params, "verifreg add verifier params").unwrap(),
    };

    apply_ok(
        v,
        &TEST_VERIFREG_ROOT_SIGNER_ADDR,
        &TEST_VERIFREG_ROOT_ADDR,
        &TokenAmount::zero(),
        MultisigMethod::Propose as u64,
        Some(proposal),
    );
    ExpectInvocation {
        from: TEST_VERIFREG_ROOT_SIGNER_ADDR,
        to: TEST_VERIFREG_ROOT_ADDR,
        method: MultisigMethod::Propose as u64,
        subinvocs: Some(vec![ExpectInvocation {
            from: TEST_VERIFREG_ROOT_ADDR,
            to: VERIFIED_REGISTRY_ACTOR_ADDR,
            method: VerifregMethod::AddVerifier as u64,
            params: Some(IpldBlock::serialize_cbor(&add_verifier_params).unwrap()),
            subinvocs: Some(vec![Expect::frc42_balance(
                VERIFIED_REGISTRY_ACTOR_ADDR,
                DATACAP_TOKEN_ACTOR_ADDR,
                *verifier,
            )]),
            ..Default::default()
        }]),
        ..Default::default()
    }
    .matches(v.take_invocations().last().unwrap());
}

pub fn verifreg_add_client(
    v: &dyn VM,
    verifier: &Address,
    client: &Address,
    allowance: StoragePower,
) {
    let add_client_params =
        AddVerifiedClientParams { address: *client, allowance: allowance.clone() };
    apply_ok(
        v,
        verifier,
        &VERIFIED_REGISTRY_ACTOR_ADDR,
        &TokenAmount::zero(),
        VerifregMethod::AddVerifiedClient as u64,
        Some(add_client_params),
    );
    let allowance_tokens = TokenAmount::from_whole(allowance);
    ExpectInvocation {
        from: *verifier,
        to: VERIFIED_REGISTRY_ACTOR_ADDR,
        method: VerifregMethod::AddVerifiedClient as u64,
        subinvocs: Some(vec![ExpectInvocation {
            from: VERIFIED_REGISTRY_ACTOR_ADDR,
            to: DATACAP_TOKEN_ACTOR_ADDR,
            method: DataCapMethod::MintExported as u64,
            params: Some(
                IpldBlock::serialize_cbor(&MintParams {
                    to: *client,
                    amount: allowance_tokens.clone(),
                    operators: vec![STORAGE_MARKET_ACTOR_ADDR],
                })
                .unwrap(),
            ),
            subinvocs: Some(vec![Expect::frc46_receiver(
                DATACAP_TOKEN_ACTOR_ADDR,
                *client,
                DATACAP_TOKEN_ACTOR_ID,
                client.id().unwrap(),
                VERIFIED_REGISTRY_ACTOR_ID,
                allowance_tokens,
                None,
            )]),
            ..Default::default()
        }]),
        ..Default::default()
    }
    .matches(v.take_invocations().last().unwrap());
}

pub fn verifreg_extend_claim_terms(
    v: &dyn VM,
    client: &Address,
    provider: &Address,
    claim: ClaimID,
    new_term: ChainEpoch,
) {
    let params = ExtendClaimTermsParams {
        terms: vec![ClaimTerm {
            provider: provider.id().unwrap(),
            claim_id: claim,
            term_max: new_term,
        }],
    };
    apply_ok(
        v,
        client,
        &VERIFIED_REGISTRY_ACTOR_ADDR,
        &TokenAmount::zero(),
        VerifregMethod::ExtendClaimTerms as u64,
        Some(params),
    );
}

pub fn verifreg_remove_expired_allocations(
    v: &dyn VM,
    caller: &Address,
    client: &Address,
    ids: Vec<AllocationID>,
    datacap_refund: u64,
) {
    let params =
        RemoveExpiredAllocationsParams { client: client.id().unwrap(), allocation_ids: ids };
    apply_ok(
        v,
        caller,
        &VERIFIED_REGISTRY_ACTOR_ADDR,
        &TokenAmount::zero(),
        VerifregMethod::RemoveExpiredAllocations as u64,
        Some(params),
    );
    ExpectInvocation {
        from: *caller,
        to: VERIFIED_REGISTRY_ACTOR_ADDR,
        method: VerifregMethod::RemoveExpiredAllocations as u64,
        subinvocs: Some(vec![ExpectInvocation {
            from: VERIFIED_REGISTRY_ACTOR_ADDR,
            to: DATACAP_TOKEN_ACTOR_ADDR,
            method: DataCapMethod::TransferExported as u64,
            params: Some(
                IpldBlock::serialize_cbor(&TransferParams {
                    to: *client,
                    amount: TokenAmount::from_whole(datacap_refund),
                    operator_data: Default::default(),
                })
                .unwrap(),
            ),
            subinvocs: Some(vec![Expect::frc46_receiver(
                DATACAP_TOKEN_ACTOR_ADDR,
                *client,
                VERIFIED_REGISTRY_ACTOR_ID,
                client.id().unwrap(),
                VERIFIED_REGISTRY_ACTOR_ID,
                TokenAmount::from_whole(datacap_refund),
                None,
            )]),
            ..Default::default()
        }]),
        ..Default::default()
    }
    .matches(v.take_invocations().last().unwrap());
}

pub fn datacap_get_balance(v: &dyn VM, address: &Address) -> TokenAmount {
    let ret = apply_ok(
        v,
        address,
        &DATACAP_TOKEN_ACTOR_ADDR,
        &TokenAmount::zero(),
        DataCapMethod::BalanceExported as u64,
        Some(address),
    );
    deserialize(&ret, "balance of return value").unwrap()
}

pub fn datacap_extend_claim(
    v: &dyn VM,
    client: &Address,
    provider: &Address,
    claim: ClaimID,
    size: u64,
    new_term: ChainEpoch,
) {
    let payload = AllocationRequests {
        allocations: vec![],
        extensions: vec![ClaimExtensionRequest {
            provider: provider.id().unwrap(),
            claim,
            term_max: new_term,
        }],
    };
    let token_amount = TokenAmount::from_whole(size);
    let operator_data = serialize(&payload, "allocation requests").unwrap();
    let transfer_params = TransferParams {
        to: VERIFIED_REGISTRY_ACTOR_ADDR,
        amount: token_amount.clone(),
        operator_data: operator_data.clone(),
    };

    apply_ok(
        v,
        client,
        &DATACAP_TOKEN_ACTOR_ADDR,
        &TokenAmount::zero(),
        DataCapMethod::TransferExported as u64,
        Some(transfer_params),
    );

    ExpectInvocation {
        from: *client,
        to: DATACAP_TOKEN_ACTOR_ADDR,
        method: DataCapMethod::TransferExported as u64,
        subinvocs: Some(vec![ExpectInvocation {
            from: DATACAP_TOKEN_ACTOR_ADDR,
            to: VERIFIED_REGISTRY_ACTOR_ADDR,
            method: VerifregMethod::UniversalReceiverHook as u64,
            params: Some(
                IpldBlock::serialize_cbor(&UniversalReceiverParams {
                    type_: FRC46_TOKEN_TYPE,
                    payload: serialize(
                        &FRC46TokenReceived {
                            from: client.id().unwrap(),
                            to: VERIFIED_REGISTRY_ACTOR_ADDR.id().unwrap(),
                            operator: client.id().unwrap(),
                            amount: token_amount.clone(),
                            operator_data,
                            token_data: RawBytes::default(),
                        },
                        "token received params",
                    )
                    .unwrap(),
                })
                .unwrap(),
            ),
            subinvocs: Some(vec![Expect::frc46_burn(
                VERIFIED_REGISTRY_ACTOR_ADDR,
                DATACAP_TOKEN_ACTOR_ADDR,
                token_amount,
            )]),
            ..Default::default()
        }]),
        ..Default::default()
    }
    .matches(v.take_invocations().last().unwrap());
}

pub fn market_add_balance(
    v: &dyn VM,
    sender: &Address,
    beneficiary: &Address,
    amount: &TokenAmount,
) {
    apply_ok(
        v,
        sender,
        &STORAGE_MARKET_ACTOR_ADDR,
        amount,
        MarketMethod::AddBalance as u64,
        Some(beneficiary),
    );
}

#[allow(clippy::too_many_arguments)]
pub fn market_publish_deal(
    v: &dyn VM,
    worker: &Address,
    deal_client: &Address,
    miner_id: &Address,
    deal_label: String,
    piece_size: PaddedPieceSize,
    verified_deal: bool,
    deal_start: ChainEpoch,
    deal_lifetime: ChainEpoch,
) -> PublishStorageDealsReturn {
    let label = Label::String(deal_label.to_string());
    let proposal = DealProposal {
        piece_cid: make_piece_cid(deal_label.as_bytes()),
        piece_size,
        verified_deal,
        client: *deal_client,
        provider: *miner_id,
        label,
        start_epoch: deal_start,
        end_epoch: deal_start + deal_lifetime,
        storage_price_per_epoch: TokenAmount::from_atto((1 << 20) as u64),
        provider_collateral: TokenAmount::from_whole(2),
        client_collateral: TokenAmount::from_whole(1),
    };

    let signature = Signature {
        sig_type: SignatureType::BLS,
        bytes: serialize(&proposal, "deal proposal").unwrap().to_vec(),
    };
    let publish_params = PublishStorageDealsParams {
        deals: vec![ClientDealProposal {
            proposal: proposal.clone(),
            client_signature: signature.clone(),
        }],
    };
    let ret: PublishStorageDealsReturn = apply_ok(
        v,
        worker,
        &STORAGE_MARKET_ACTOR_ADDR,
        &TokenAmount::zero(),
        MarketMethod::PublishStorageDeals as u64,
        Some(publish_params),
    )
    .deserialize()
    .unwrap();

    let proposal_bytes = serialize(&proposal, "deal proposal").unwrap();

    let mut expect_publish_invocs = vec![
        ExpectInvocation {
            from: STORAGE_MARKET_ACTOR_ADDR,
            to: *miner_id,
            method: MinerMethod::IsControllingAddressExported as u64,
            ..Default::default()
        },
        Expect::reward_this_epoch(STORAGE_MARKET_ACTOR_ADDR),
        Expect::power_current_total(STORAGE_MARKET_ACTOR_ADDR),
        Expect::frc44_authenticate(
            STORAGE_MARKET_ACTOR_ADDR,
            *deal_client,
            proposal_bytes.to_vec(),
            signature.bytes,
        ),
    ];
    if verified_deal {
        let deal_term = proposal.end_epoch - proposal.start_epoch;
        let token_amount = TokenAmount::from_whole(proposal.piece_size.0 as i64);
        let alloc_expiration =
            min(proposal.start_epoch, v.epoch() + MAXIMUM_VERIFIED_ALLOCATION_EXPIRATION);

        expect_publish_invocs.push(ExpectInvocation {
            from: STORAGE_MARKET_ACTOR_ADDR,
            to: DATACAP_TOKEN_ACTOR_ADDR,
            method: DataCapMethod::BalanceExported as u64,
            params: Some(IpldBlock::serialize_cbor(&deal_client).unwrap()),
            ..Default::default()
        });
        let alloc_reqs = AllocationRequests {
            allocations: vec![AllocationRequest {
                provider: miner_id.id().unwrap(),
                data: proposal.piece_cid,
                size: proposal.piece_size,
                term_min: deal_term,
                term_max: deal_term + MARKET_DEFAULT_ALLOCATION_TERM_BUFFER,
                expiration: alloc_expiration,
            }],
            extensions: vec![],
        };
        expect_publish_invocs.push(ExpectInvocation {
            from: STORAGE_MARKET_ACTOR_ADDR,
            to: DATACAP_TOKEN_ACTOR_ADDR,
            method: DataCapMethod::TransferFromExported as u64,
            params: Some(
                IpldBlock::serialize_cbor(&TransferFromParams {
                    from: *deal_client,
                    to: VERIFIED_REGISTRY_ACTOR_ADDR,
                    amount: token_amount.clone(),
                    operator_data: RawBytes::serialize(&alloc_reqs).unwrap(),
                })
                .unwrap(),
            ),
            subinvocs: Some(vec![ExpectInvocation {
                from: DATACAP_TOKEN_ACTOR_ADDR,
                to: VERIFIED_REGISTRY_ACTOR_ADDR,
                method: VerifregMethod::UniversalReceiverHook as u64,
                params: Some(
                    IpldBlock::serialize_cbor(&UniversalReceiverParams {
                        type_: FRC46_TOKEN_TYPE,
                        payload: serialize(
                            &FRC46TokenReceived {
                                from: deal_client.id().unwrap(),
                                to: VERIFIED_REGISTRY_ACTOR_ADDR.id().unwrap(),
                                operator: STORAGE_MARKET_ACTOR_ADDR.id().unwrap(),
                                amount: token_amount,
                                operator_data: RawBytes::serialize(&alloc_reqs).unwrap(),
                                token_data: Default::default(),
                            },
                            "token received params",
                        )
                        .unwrap(),
                    })
                    .unwrap(),
                ),
                ..Default::default()
            }]),
            ..Default::default()
        })
    }
    expect_publish_invocs.push(ExpectInvocation {
        from: STORAGE_MARKET_ACTOR_ADDR,
        to: *deal_client,
        method: MARKET_NOTIFY_DEAL_METHOD,
        ..Default::default()
    });
    ExpectInvocation {
        from: *worker,
        to: STORAGE_MARKET_ACTOR_ADDR,
        method: MarketMethod::PublishStorageDeals as u64,
        subinvocs: Some(expect_publish_invocs),
        ..Default::default()
    }
    .matches(v.take_invocations().last().unwrap());

    ret
}

pub fn make_bitfield(bits: &[u64]) -> BitField {
    BitField::try_from_bits(bits.iter().copied()).unwrap()
}

pub fn bf_all(bf: BitField) -> Vec<u64> {
    bf.bounded_iter(Policy::default().addressed_sectors_max).unwrap().collect()
}

pub mod invariant_failure_patterns {
    use lazy_static::lazy_static;
    use regex::Regex;

    lazy_static! {
        pub static ref REWARD_STATE_EPOCH_MISMATCH: Regex =
            Regex::new("^reward state epoch \\d+ does not match prior_epoch\\+1 \\d+$").unwrap();
    }
}

pub fn generate_deal_proposal(
    client: &Address,
    provider: &Address,
    client_collateral: &TokenAmount,
    provider_collateral: &TokenAmount,
    start_epoch: ChainEpoch,
    end_epoch: ChainEpoch,
) -> DealProposal {
    let piece_cid = make_piece_cid("1".as_bytes());
    let piece_size = PaddedPieceSize(2048u64);
    let storage_price_per_epoch = TokenAmount::from_atto(10u8);
    DealProposal {
        piece_cid,
        piece_size,
        verified_deal: false,
        client: *client,
        provider: *provider,
        label: Label::String("label".to_string()),
        start_epoch,
        end_epoch,
        storage_price_per_epoch,
        provider_collateral: provider_collateral.clone(),
        client_collateral: client_collateral.clone(),
    }
}
