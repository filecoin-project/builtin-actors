use cid::Cid;
use std::cmp::min;

use fil_actor_market::SettleDealPaymentsParams;
use fil_actor_market::SettleDealPaymentsReturn;
use frc46_token::receiver::FRC46TokenReceived;
use frc46_token::receiver::FRC46_TOKEN_TYPE;
use frc46_token::token::types::TransferParams;
use frc46_token::token::types::{TransferFromParams, TransferReturn};
use fvm_actor_utils::receiver::UniversalReceiverParams;
use fvm_ipld_bitfield::BitField;
use fvm_ipld_blockstore::Blockstore;
use fvm_ipld_encoding::ipld_block::IpldBlock;
use fvm_ipld_encoding::BytesDe;
use fvm_ipld_encoding::RawBytes;
use fvm_shared::address::Address;
use fvm_shared::bigint::bigint_ser::BigIntSer;
use fvm_shared::clock::ChainEpoch;
use fvm_shared::crypto::signature::Signature;
use fvm_shared::crypto::signature::SignatureType;
use fvm_shared::deal::DealID;
use fvm_shared::econ::TokenAmount;
use fvm_shared::error::ExitCode;
use fvm_shared::piece::{PaddedPieceSize, PieceInfo};
use fvm_shared::randomness::Randomness;
use fvm_shared::sector::PoStProof;
use fvm_shared::sector::RegisteredPoStProof;
use fvm_shared::sector::RegisteredSealProof;
use fvm_shared::sector::SectorNumber;
use fvm_shared::sector::StoragePower;
use num_traits::Zero;

use fil_actor_cron::Method as CronMethod;
use fil_actor_datacap::Method as DataCapMethod;
use fil_actor_market::ext::verifreg::AllocationsResponse;
use fil_actor_market::{
    ClientDealProposal, DealProposal, Label, Method as MarketMethod, PublishStorageDealsParams,
    PublishStorageDealsReturn, SectorDeals, State as MarketState, MARKET_NOTIFY_DEAL_METHOD,
};
use fil_actor_miner::{
    aggregate_pre_commit_network_fee, aggregate_prove_commit_network_fee,
    max_prove_commit_duration, ChangeBeneficiaryParams, CompactCommD, DataActivationNotification,
    DeadlineInfo, DeclareFaultsRecoveredParams, ExpirationExtension2,
    ExtendSectorExpiration2Params, Method as MinerMethod, PieceActivationManifest, PoStPartition,
    PowerPair, PreCommitSectorBatchParams2, ProveCommitAggregateParams, ProveCommitSectors3Params,
    RecoveryDeclaration, SectorActivationManifest, SectorClaim, SectorPreCommitInfo,
    SectorPreCommitOnChainInfo, State as MinerState, SubmitWindowedPoStParams,
    VerifiedAllocationKey, WithdrawBalanceParams, WithdrawBalanceReturn,
};
use fil_actor_multisig::Method as MultisigMethod;
use fil_actor_multisig::ProposeParams;
use fil_actor_power::{CreateMinerParams, CreateMinerReturn, Method as PowerMethod};
use fil_actor_verifreg::ext::datacap::MintParams;
use fil_actor_verifreg::ClaimExtensionRequest;
use fil_actor_verifreg::{state, AllocationRequests};
use fil_actor_verifreg::{
    AddVerifiedClientParams, AllocationID, ClaimID, ClaimTerm, ExtendClaimTermsParams,
    Method as VerifregMethod, RemoveExpiredAllocationsParams, State as VerifregState,
    VerifierParams,
};
use fil_actor_verifreg::{AllocationRequest, DataCap};
use fil_actors_runtime::cbor::deserialize;
use fil_actors_runtime::cbor::serialize;
use fil_actors_runtime::runtime::policy_constants::{
    MARKET_DEFAULT_ALLOCATION_TERM_BUFFER, MAXIMUM_VERIFIED_ALLOCATION_EXPIRATION,
};
use fil_actors_runtime::runtime::Policy;
use fil_actors_runtime::test_utils::make_piece_cid;
use fil_actors_runtime::test_utils::make_sealed_cid;
use fil_actors_runtime::DealWeight;
use fil_actors_runtime::EventBuilder;
use fil_actors_runtime::CRON_ACTOR_ADDR;
use fil_actors_runtime::DATACAP_TOKEN_ACTOR_ADDR;
use fil_actors_runtime::STORAGE_MARKET_ACTOR_ADDR;
use fil_actors_runtime::STORAGE_MARKET_ACTOR_ID;
use fil_actors_runtime::STORAGE_POWER_ACTOR_ADDR;
use fil_actors_runtime::SYSTEM_ACTOR_ADDR;
use fil_actors_runtime::VERIFIED_REGISTRY_ACTOR_ADDR;
use fil_actors_runtime::{DATACAP_TOKEN_ACTOR_ID, VERIFIED_REGISTRY_ACTOR_ID};
use vm_api::trace::{EmittedEvent, ExpectInvocation};
use vm_api::util::get_state;
use vm_api::util::DynBlockstore;
use vm_api::util::{apply_code, apply_ok, apply_ok_implicit};
use vm_api::VM;

use crate::expects::Expect;
use crate::*;

use super::make_bitfield;
use super::market_pending_deal_allocations_raw;
use super::miner_dline_info;
use super::sector_deadline;

pub fn cron_tick(v: &dyn VM) {
    apply_ok_implicit(
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

#[allow(clippy::too_many_arguments)]
pub fn miner_precommit_one_sector_v2(
    v: &dyn VM,
    worker: &Address,
    maddr: &Address,
    seal_proof: RegisteredSealProof,
    sector_number: SectorNumber,
    meta_data: PrecommitMetadata,
    expect_cron_enroll: bool,
    expiration: ChainEpoch,
) -> SectorPreCommitOnChainInfo {
    precommit_sectors_v2(
        v,
        1,
        1,
        vec![meta_data],
        worker,
        maddr,
        seal_proof,
        sector_number,
        expect_cron_enroll,
        Some(expiration),
    )[0]
    .clone()
}

pub fn miner_prove_sector(
    v: &dyn VM,
    worker: &Address,
    miner_id: &Address,
    sector_number: SectorNumber,
    manifests: Vec<PieceActivationManifest>,
) {
    let prove_commit_params = ProveCommitSectors3Params {
        sector_activations: vec![SectorActivationManifest { sector_number, pieces: manifests }],
        sector_proofs: vec![vec![].into()],
        aggregate_proof: RawBytes::default(),
        aggregate_proof_type: None,
        require_activation_success: true,
        require_notification_success: true,
    };
    apply_ok(
        v,
        worker,
        miner_id,
        &TokenAmount::zero(),
        MinerMethod::ProveCommitSectors3 as u64,
        Some(prove_commit_params),
    );

    let worker_id = v.resolve_id_address(worker).unwrap().id().unwrap();

    ExpectInvocation {
        from: worker_id,
        to: *miner_id,
        method: MinerMethod::ProveCommitSectors3 as u64,
        ..Default::default()
    }
    .matches(v.take_invocations().last().unwrap());
}

#[derive(Default, Clone)]
pub struct PrecommitMetadata {
    pub deals: Vec<DealID>,
    pub commd: CompactCommD,
}

#[allow(clippy::too_many_arguments)]
pub fn precommit_sectors_v2_expect_code(
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
    code: ExitCode,
) {
    let miner_id_address = v.resolve_id_address(maddr).unwrap();
    let miner_id = miner_id_address.id().unwrap();
    let worker_id = v.resolve_id_address(worker).unwrap().id().unwrap();
    let expiration = match exp {
        None => {
            v.epoch()
                + Policy::default().min_sector_expiration
                + max_prove_commit_duration(&Policy::default(), seal_proof).unwrap()
        }
        Some(e) => e,
    };

    let mut sector_idx: usize = 0;
    let no_deals = PrecommitMetadata::default();
    let mut sectors_with_deals: Vec<SectorDeals> = vec![];
    while sector_idx < count {
        let msg_sector_idx_base = sector_idx;
        let mut invocs =
            vec![Expect::reward_this_epoch(miner_id), Expect::power_current_total(miner_id)];
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
                    sector_number,
                    sector_type: seal_proof,
                    sector_expiry: expiration,
                    deal_ids: sector_meta.deals.clone(),
                });
            }
            sector_idx += 1;
            j += 1;
        }

        let events: Vec<EmittedEvent> = param_sectors
            .iter()
            .map(|ps| Expect::build_miner_event("sector-precommitted", miner_id, ps.sector_number))
            .collect();

        if !sectors_with_deals.is_empty() {
            invocs.push(Expect::market_verify_deals(miner_id, sectors_with_deals.clone()));
        }
        if param_sectors.len() > 1 {
            invocs.push(Expect::burn(
                miner_id,
                Some(aggregate_pre_commit_network_fee(param_sectors.len(), &TokenAmount::zero())),
            ));
        }
        if expect_cron_enroll && msg_sector_idx_base == 0 {
            invocs.push(Expect::power_enrol_cron(miner_id));
        }

        apply_code(
            v,
            worker,
            maddr,
            &TokenAmount::zero(),
            MinerMethod::PreCommitSectorBatch2 as u64,
            Some(PreCommitSectorBatchParams2 { sectors: param_sectors.clone() }),
            code,
        );
        if code == ExitCode::OK {
            let expect = ExpectInvocation {
                from: worker_id,
                to: miner_id_address,
                method: MinerMethod::PreCommitSectorBatch2 as u64,
                params: Some(
                    IpldBlock::serialize_cbor(&PreCommitSectorBatchParams2 {
                        sectors: param_sectors,
                    })
                    .unwrap(),
                ),
                subinvocs: Some(invocs),
                events: Some(events),
                ..Default::default()
            };
            expect.matches(v.take_invocations().last().unwrap());
        }
    }
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
) -> Vec<SectorPreCommitOnChainInfo> {
    let mid = v.resolve_id_address(maddr).unwrap();
    precommit_sectors_v2_expect_code(
        v,
        count,
        batch_size,
        metadata,
        worker,
        maddr,
        seal_proof,
        sector_number_base,
        expect_cron_enroll,
        exp,
        ExitCode::OK,
    );

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

pub fn precommit_meta_data_from_deals(
    v: &dyn VM,
    deal_ids: &[u64],
    seal_proof: RegisteredSealProof,
    include_ids: bool,
) -> PrecommitMetadata {
    let state: MarketState = get_state(v, &STORAGE_MARKET_ACTOR_ADDR).unwrap();
    let pieces: Vec<PieceInfo> = deal_ids
        .iter()
        .map(|id: &u64| {
            let deal = state.get_proposal(&DynBlockstore::wrap(v.blockstore()), *id).unwrap();
            PieceInfo { size: deal.piece_size, cid: deal.piece_cid }
        })
        .collect();

    let ids = if include_ids { deal_ids.to_vec() } else { vec![] };
    PrecommitMetadata {
        deals: ids,
        commd: CompactCommD::of(
            v.primitives().compute_unsealed_sector_cid(seal_proof, &pieces).unwrap(),
        ),
    }
}

pub fn prove_commit_sectors(
    v: &dyn VM,
    worker: &Address,
    maddr: &Address,
    precommits: Vec<SectorPreCommitOnChainInfo>,
    aggregate_size: usize,
) {
    let worker_id = v.resolve_id_address(worker).unwrap().id().unwrap();
    let miner_id = v.resolve_id_address(maddr).unwrap().id().unwrap();
    let mut precommit_infos = precommits.as_slice();
    while !precommit_infos.is_empty() {
        let batch_size = min(aggregate_size, precommit_infos.len());
        let to_prove = &precommit_infos[0..batch_size];
        precommit_infos = &precommit_infos[batch_size..];
        let b: Vec<u64> = to_prove.iter().map(|p| p.info.sector_number).collect();

        let prove_commit_aggregate_params = ProveCommitAggregateParams {
            sector_numbers: make_bitfield(b.as_slice()),
            aggregate_proof: vec![].into(),
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

        let st: MarketState = get_state(v, &STORAGE_MARKET_ACTOR_ADDR).unwrap();
        let store = DynBlockstore::wrap(v.blockstore());
        let events: Vec<EmittedEvent> = to_prove
            .iter()
            .map(|ps| {
                let mut pieces: Vec<(Cid, u64)> = vec![];
                for deal_id in &ps.info.deal_ids {
                    let proposal = st.get_proposal(&store, *deal_id).unwrap();
                    pieces.push((proposal.piece_cid, proposal.piece_size.0));
                }

                let unsealed_cid = ps.info.unsealed_cid.0;
                Expect::build_sector_activation_event(
                    "sector-activated",
                    miner_id,
                    ps.info.sector_number,
                    unsealed_cid,
                    &pieces,
                )
            })
            .collect();

        let expected_fee = aggregate_prove_commit_network_fee(to_prove.len(), &TokenAmount::zero());
        ExpectInvocation {
            from: worker_id,
            to: *maddr,
            method: MinerMethod::ProveCommitAggregate as u64,
            params: Some(prove_commit_aggregate_params_ser),
            subinvocs: Some(vec![
                Expect::reward_this_epoch(miner_id),
                Expect::power_current_total(miner_id),
                Expect::power_update_pledge(miner_id, None),
                Expect::burn(miner_id, Some(expected_fee)),
            ]),
            events: Some(events),
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
    let miner_id = miner.id().unwrap();
    let worker_id = worker.id().unwrap();
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
        subinvocs.push(Expect::verifreg_get_claims(miner_id, miner_id, claim_ids))
    }
    subinvocs.push(Expect::reward_this_epoch(miner_id));
    subinvocs.push(Expect::power_current_total(miner_id));
    if !power_delta.is_zero() {
        subinvocs.push(Expect::power_update_claim(miner_id, power_delta));
    }

    ExpectInvocation {
        from: worker_id,
        to: *miner,
        method: MinerMethod::ExtendSectorExpiration2 as u64,
        subinvocs: Some(subinvocs),
        ..Default::default()
    }
    .matches(v.take_invocations().last().unwrap());
}

pub fn provider_settle_deal_payments(
    v: &dyn VM,
    provider: &Address,
    deals: &[DealID],
) -> SettleDealPaymentsReturn {
    let mut deal_id_bitfield = BitField::new();
    for deal_id in deals {
        deal_id_bitfield.set(*deal_id);
    }
    let params = SettleDealPaymentsParams { deal_ids: deal_id_bitfield };
    let ret = apply_ok(
        v,
        provider,
        &STORAGE_MARKET_ACTOR_ADDR,
        &TokenAmount::zero(),
        MarketMethod::SettleDealPaymentsExported as u64,
        Some(params),
    );
    ret.deserialize::<SettleDealPaymentsReturn>().unwrap()
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
    let miner_id = maddr.id().unwrap();
    let worker_id = worker.id().unwrap();
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
            subinvocs = Some(vec![Expect::power_update_claim(miner_id, new_pow)])
        }
    }

    ExpectInvocation {
        from: worker_id,
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
    let from_id = v.resolve_id_address(from).unwrap().id().unwrap();
    let miner_id = v.resolve_id_address(m_addr).unwrap().id().unwrap();
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
            from: from_id,
            to: *m_addr,
            method: MinerMethod::WithdrawBalance as u64,
            params: Some(withdraw_balance_params_se),
            subinvocs: Some(vec![Expect::send(
                miner_id,
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

pub fn verifier_balance_event(verifier: ActorID, data_cap: DataCap) -> EmittedEvent {
    EmittedEvent {
        emitter: VERIFIED_REGISTRY_ACTOR_ID,
        event: EventBuilder::new()
            .typ("verifier-balance")
            .field_indexed("verifier", &verifier)
            .field("balance", &BigIntSer(&data_cap))
            .build()
            .unwrap(),
    }
}

pub fn verifier_balance_event_with_client(
    verifier: ActorID,
    data_cap: DataCap,
    client: ActorID,
) -> EmittedEvent {
    EmittedEvent {
        emitter: VERIFIED_REGISTRY_ACTOR_ID,
        event: EventBuilder::new()
            .typ("verifier-balance")
            .field_indexed("verifier", &verifier)
            .field("balance", &BigIntSer(&data_cap))
            .field_indexed("client", &client)
            .build()
            .unwrap(),
    }
}

pub fn verifreg_add_verifier(v: &dyn VM, verifier: &Address, data_cap: StoragePower) {
    let add_verifier_params = VerifierParams { address: *verifier, allowance: data_cap.clone() };
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
        from: TEST_VERIFREG_ROOT_SIGNER_ID,
        to: TEST_VERIFREG_ROOT_ADDR,
        method: MultisigMethod::Propose as u64,
        subinvocs: Some(vec![ExpectInvocation {
            from: TEST_VERIFREG_ROOT_ID,
            to: VERIFIED_REGISTRY_ACTOR_ADDR,
            method: VerifregMethod::AddVerifier as u64,
            params: Some(IpldBlock::serialize_cbor(&add_verifier_params).unwrap()),
            subinvocs: Some(vec![Expect::frc42_balance(
                VERIFIED_REGISTRY_ACTOR_ID,
                DATACAP_TOKEN_ACTOR_ADDR,
                *verifier,
            )]),
            events: Some(vec![verifier_balance_event(verifier.id().unwrap(), data_cap)]),
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
    let v_st: VerifregState = get_state(v, &VERIFIED_REGISTRY_ACTOR_ADDR).unwrap();
    let store = DynBlockstore::wrap(v.blockstore());

    let verifier_cap = v_st.get_verifier_cap(&store, verifier).unwrap().unwrap();

    let updated_verifier_balance = verifier_cap - allowance.clone();

    let verifier_id = v.resolve_id_address(verifier).unwrap().id().unwrap();
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
        from: verifier_id,
        to: VERIFIED_REGISTRY_ACTOR_ADDR,
        method: VerifregMethod::AddVerifiedClient as u64,
        subinvocs: Some(vec![ExpectInvocation {
            from: VERIFIED_REGISTRY_ACTOR_ID,
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
                DATACAP_TOKEN_ACTOR_ID,
                *client,
                DATACAP_TOKEN_ACTOR_ID,
                client.id().unwrap(),
                VERIFIED_REGISTRY_ACTOR_ID,
                allowance_tokens,
                None,
            )]),
            ..Default::default()
        }]),
        events: Some(vec![verifier_balance_event_with_client(
            verifier.id().unwrap(),
            updated_verifier_balance,
            client.id().unwrap(),
        )]),
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
    expected_expirations: Vec<AllocationID>,
) {
    let v_st: VerifregState = get_state(v, &VERIFIED_REGISTRY_ACTOR_ADDR).unwrap();
    let store = DynBlockstore::wrap(v.blockstore());
    let mut allocs = v_st.load_allocs(&store).unwrap();
    let expected_events: Vec<EmittedEvent> = expected_expirations
        .iter()
        .map(|id| {
            let alloc = allocs.get(client.id().unwrap(), *id).unwrap().unwrap();
            Expect::build_verifreg_allocation_event(
                "allocation-removed",
                *id,
                client.id().unwrap(),
                alloc.provider,
                &alloc.data,
                alloc.size.0,
                alloc.term_min,
                alloc.term_max,
                alloc.expiration,
            )
        })
        .collect();

    let caller_id = v.resolve_id_address(caller).unwrap().id().unwrap();
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
        from: caller_id,
        to: VERIFIED_REGISTRY_ACTOR_ADDR,
        method: VerifregMethod::RemoveExpiredAllocations as u64,
        subinvocs: Some(vec![ExpectInvocation {
            from: VERIFIED_REGISTRY_ACTOR_ID,
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
                DATACAP_TOKEN_ACTOR_ID,
                *client,
                VERIFIED_REGISTRY_ACTOR_ID,
                client.id().unwrap(),
                VERIFIED_REGISTRY_ACTOR_ID,
                TokenAmount::from_whole(datacap_refund),
                None,
            )]),
            ..Default::default()
        }]),
        events: Some(expected_events),
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

pub fn datacap_create_allocations(
    v: &dyn VM,
    client: &Address,
    reqs: &[AllocationRequest],
) -> Vec<AllocationID> {
    let payload = AllocationRequests { allocations: reqs.to_vec(), extensions: vec![] };
    let token_amount = TokenAmount::from_whole(reqs.iter().map(|r| r.size.0).sum::<u64>());
    let operator_data = serialize(&payload, "allocation requests").unwrap();
    let transfer_params = TransferParams {
        to: VERIFIED_REGISTRY_ACTOR_ADDR,
        amount: token_amount.clone(),
        operator_data: operator_data.clone(),
    };

    let client_id = v.resolve_id_address(client).unwrap().id().unwrap();
    let tfer_result: TransferReturn = apply_ok(
        v,
        client,
        &DATACAP_TOKEN_ACTOR_ADDR,
        &TokenAmount::zero(),
        DataCapMethod::TransferExported as u64,
        Some(transfer_params),
    )
    .deserialize()
    .unwrap();
    let alloc_response: AllocationsResponse = tfer_result.recipient_data.deserialize().unwrap();

    let events: Vec<EmittedEvent> = alloc_response
        .new_allocations
        .iter()
        .enumerate()
        .map(|(i, alloc_id)| {
            Expect::build_verifreg_allocation_event(
                "allocation",
                *alloc_id,
                client.id().unwrap(),
                reqs[i].provider,
                &reqs[i].data,
                reqs[i].size.0,
                reqs[i].term_min,
                reqs[i].term_max,
                reqs[i].expiration,
            )
        })
        .collect::<Vec<EmittedEvent>>();

    Expect::datacap_transfer_to_verifreg(
        client_id,
        token_amount,
        operator_data,
        false, // No burn
        events,
    )
    .matches(v.take_invocations().last().unwrap());

    alloc_response.new_allocations
}

pub fn datacap_extend_claim(
    v: &dyn VM,
    client: &Address,
    provider: &Address,
    claim: ClaimID,
    size: u64,
    new_term: ChainEpoch,
) {
    // read existing claim with claim id from VerifReg state
    let v_st: fil_actor_verifreg::State = get_state(v, &VERIFIED_REGISTRY_ACTOR_ADDR).unwrap();
    let store = DynBlockstore::wrap(v.blockstore());
    let mut claims = v_st.load_claims(&store).unwrap();
    let mut existing_claim =
        state::get_claim(&mut claims, provider.id().unwrap(), claim).unwrap().unwrap().clone();
    existing_claim.term_max = new_term;

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

    let client_id = v.resolve_id_address(client).unwrap().id().unwrap();
    let claim_updated_event = Expect::build_verifreg_claim_event(
        "claim-updated",
        claim,
        existing_claim.client,
        provider.id().unwrap(),
        &existing_claim.data,
        existing_claim.size.0,
        existing_claim.term_min,
        existing_claim.term_max,
        existing_claim.term_start,
        existing_claim.sector,
    );

    Expect::datacap_transfer_to_verifreg(
        client_id,
        token_amount,
        operator_data,
        true, // Burn
        vec![claim_updated_event],
    )
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
    let worker_id = v.resolve_id_address(worker).unwrap().id().unwrap();
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
            from: STORAGE_MARKET_ACTOR_ID,
            to: *miner_id,
            method: MinerMethod::IsControllingAddressExported as u64,
            ..Default::default()
        },
        Expect::reward_this_epoch(STORAGE_MARKET_ACTOR_ID),
        Expect::power_current_total(STORAGE_MARKET_ACTOR_ID),
        Expect::frc44_authenticate(
            STORAGE_MARKET_ACTOR_ID,
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
            from: STORAGE_MARKET_ACTOR_ID,
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

        let v_st: fil_actor_verifreg::State = get_state(v, &VERIFIED_REGISTRY_ACTOR_ADDR).unwrap();
        let alloc_id = v_st.next_allocation_id - 1;
        let alloc_req = alloc_reqs.allocations[0].clone();
        let alloc_event = Expect::build_verifreg_allocation_event(
            "allocation",
            alloc_id,
            deal_client.id().unwrap(),
            miner_id.id().unwrap(),
            &proposal.piece_cid,
            proposal.piece_size.0,
            alloc_req.term_min,
            alloc_req.term_max,
            alloc_req.expiration,
        );

        expect_publish_invocs.push(ExpectInvocation {
            from: STORAGE_MARKET_ACTOR_ID,
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
                from: DATACAP_TOKEN_ACTOR_ID,
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
                events: Some(vec![alloc_event]),
                ..Default::default()
            }]),
            ..Default::default()
        })
    }
    expect_publish_invocs.push(ExpectInvocation {
        from: STORAGE_MARKET_ACTOR_ID,
        to: *deal_client,
        method: MARKET_NOTIFY_DEAL_METHOD,
        ..Default::default()
    });
    ExpectInvocation {
        from: worker_id,
        to: STORAGE_MARKET_ACTOR_ADDR,
        method: MarketMethod::PublishStorageDeals as u64,
        subinvocs: Some(expect_publish_invocs),
        events: Some(vec![Expect::build_market_event(
            "deal-published",
            ret.ids[0],
            deal_client.id().unwrap(),
            miner_id.id().unwrap(),
        )]),
        ..Default::default()
    }
    .matches(v.take_invocations().last().unwrap());

    ret
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

pub fn get_deal(v: &dyn VM, deal_id: DealID) -> DealProposal {
    let actor = v.actor(&STORAGE_MARKET_ACTOR_ADDR).unwrap();
    let bs = DynBlockstore::wrap(v.blockstore());
    let state: fil_actor_market::State =
        RawBytes::new(bs.get(&actor.state).unwrap().unwrap()).deserialize().unwrap();
    state.get_proposal(&bs, deal_id).unwrap()
}

// return (deal_weight, verified_deal_weight)
pub fn get_deal_weights(
    v: &dyn VM,
    deal_id: DealID,
    duration: ChainEpoch,
) -> (DealWeight, DealWeight) {
    let deal = get_deal(v, deal_id);
    if deal.verified_deal {
        return (DealWeight::zero(), DealWeight::from(deal.piece_size.0 * duration as u64));
    }
    (DealWeight::from(deal.piece_size.0 * duration as u64), DealWeight::zero())
}

pub fn make_piece_manifests_from_deal_ids(
    v: &dyn VM,
    deal_ids: Vec<DealID>,
) -> Vec<PieceActivationManifest> {
    let mut piece_manifests = vec![];
    for deal_id in deal_ids {
        let deal = get_deal(v, deal_id);
        let alloc_key = match market_pending_deal_allocations_raw(v, &[deal_id]) {
            Ok(alloc_ids) => Some(VerifiedAllocationKey {
                id: *alloc_ids.first().unwrap(),
                client: deal.client.id().unwrap(),
            }),
            Err(_) => None,
        };

        piece_manifests.push(PieceActivationManifest {
            cid: deal.piece_cid,
            size: deal.piece_size,
            verified_allocation_key: alloc_key,
            notify: vec![DataActivationNotification {
                address: STORAGE_MARKET_ACTOR_ADDR,
                payload: serialize(&deal_id, "dealid").unwrap(),
            }],
        });
    }
    piece_manifests
}
