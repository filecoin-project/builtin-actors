use cid::Cid;
use std::cmp::min;

use fil_actor_market::SettleDealPaymentsParams;
use fil_actor_market::SettleDealPaymentsReturn;
use fvm_ipld_bitfield::BitField;
use fvm_ipld_blockstore::Blockstore;
use fvm_ipld_encoding::BytesDe;
use fvm_ipld_encoding::RawBytes;
use fvm_ipld_encoding::ipld_block::IpldBlock;
use fvm_shared::address::Address;
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
use num_traits::Zero;

use fil_actor_cron::Method as CronMethod;
use fil_actor_datacap::Method as DataCapMethod;
use fil_actor_market::{
    ClientDealProposal, DealProposal, Label, MARKET_NOTIFY_DEAL_METHOD, Method as MarketMethod,
    PublishStorageDealsParams, PublishStorageDealsReturn, SectorDeals, State as MarketState,
};
use fil_actor_miner::{
    ChangeBeneficiaryParams, CompactCommD, DataActivationNotification, DeadlineInfo,
    DeclareFaultsRecoveredParams, ExpirationExtension2, ExtendSectorExpiration2Params,
    Method as MinerMethod, PieceActivationManifest, PoStPartition, PowerPair,
    PreCommitSectorBatchParams2, ProveCommitSectors3Params, RecoveryDeclaration,
    SectorActivationManifest, SectorClaim, SectorPreCommitInfo, SectorPreCommitOnChainInfo,
    State as MinerState, SubmitWindowedPoStParams, WithdrawBalanceParams, WithdrawBalanceReturn,
    max_prove_commit_duration,
};
use fil_actor_power::{CreateMinerParams, CreateMinerReturn, Method as PowerMethod};
use fil_actors_runtime::CRON_ACTOR_ADDR;
use fil_actors_runtime::DATACAP_TOKEN_ACTOR_ADDR;
use fil_actors_runtime::DealWeight;
use fil_actors_runtime::STORAGE_MARKET_ACTOR_ADDR;
use fil_actors_runtime::STORAGE_MARKET_ACTOR_ID;
use fil_actors_runtime::STORAGE_POWER_ACTOR_ADDR;
use fil_actors_runtime::SYSTEM_ACTOR_ADDR;
use fil_actors_runtime::cbor::deserialize;
use fil_actors_runtime::cbor::serialize;
use fil_actors_runtime::runtime::Policy;
use fil_actors_runtime::test_utils::make_piece_cid;
use fil_actors_runtime::test_utils::make_sealed_cid;
use vm_api::VM;
use vm_api::trace::{EmittedEvent, ExpectInvocation};
use vm_api::util::DynBlockstore;
use vm_api::util::get_state;
use vm_api::util::{apply_code, apply_ok, apply_ok_implicit};

use crate::expects::Expect;
use crate::*;

use super::create_miner_deposit_for_test;

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

pub fn owner_add_create_miner_deposit(v: &dyn VM, owner: &Address) -> TokenAmount {
    let create_miner_deposit = create_miner_deposit_for_test(v);
    apply_ok(
        v,
        &TEST_FAUCET_ADDR,
        owner,
        &create_miner_deposit,
        fvm_shared::METHOD_SEND,
        None::<RawBytes>,
    );

    create_miner_deposit
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
    let res: CreateMinerReturn =
        create_miner_internal(v, &params, balance).ret.unwrap().deserialize().unwrap();
    (res.id_address, res.robust_address)
}

pub fn create_miner_internal(
    v: &dyn VM,
    params: &CreateMinerParams,
    balance: &TokenAmount,
) -> vm_api::MessageResult {
    let owner = &params.owner;
    // sent deposit to owner
    let deposit = owner_add_create_miner_deposit(v, owner);

    let params = IpldBlock::serialize_cbor(&params).unwrap().unwrap();
    let ret = v
        .execute_message(
            owner,
            &STORAGE_POWER_ACTOR_ADDR,
            &deposit,
            PowerMethod::CreateMiner as u64,
            Some(params),
        )
        .unwrap();
    let res: CreateMinerReturn = ret.ret.as_ref().unwrap().deserialize().unwrap();

    let wrap_store = DynBlockstore::wrap(v.blockstore());
    vm_api::util::mutate_state(v, &res.id_address, |st: &mut MinerState| {
        // checkcreate miner deposit
        assert!(st.vesting_funds.load(&wrap_store).unwrap().len() == 180);
        assert!(st.locked_funds == deposit);

        // reset create miner deposit vesting funds
        st.vesting_funds = Default::default();
        st.locked_funds = TokenAmount::zero();
    });

    let state: MinerState = get_state(v, &res.id_address).unwrap();
    assert!(state.vesting_funds.load(&wrap_store).unwrap().is_empty());
    assert!(state.locked_funds.is_zero());

    let mut actor_state = v.actor(&res.id_address).unwrap();
    actor_state.balance = balance.clone();
    v.set_actor(&res.id_address, actor_state);

    let actual_balance = v.balance(&res.id_address);
    assert_eq!(&actual_balance, balance);

    ret
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
        while sector_idx < count {
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
        }

        let events: Vec<EmittedEvent> = param_sectors
            .iter()
            .map(|ps| Expect::build_miner_event("sector-precommitted", miner_id, ps.sector_number))
            .collect();

        if !sectors_with_deals.is_empty() {
            invocs.push(Expect::market_verify_deals(miner_id, sectors_with_deals.clone()));
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

        let sector_activations: Vec<SectorActivationManifest> = to_prove
            .iter()
            .map(|p| SectorActivationManifest {
                sector_number: p.info.sector_number,
                pieces: vec![],
            })
            .collect();

        let prove_commit_params = ProveCommitSectors3Params {
            sector_activations: sector_activations.clone(),
            sector_proofs: sector_activations
                .iter()
                .map(|sa| RawBytes::new(vec![sa.sector_number as u8; 4]))
                .collect(),
            aggregate_proof: vec![].into(),
            aggregate_proof_type: None,
            require_activation_success: true,
            require_notification_success: false,
        };

        let prove_commit_params_ser = IpldBlock::serialize_cbor(&prove_commit_params).unwrap();

        apply_ok(
            v,
            worker,
            maddr,
            &TokenAmount::zero(),
            MinerMethod::ProveCommitSectors3 as u64,
            Some(prove_commit_params),
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

        ExpectInvocation {
            from: worker_id,
            to: *maddr,
            method: MinerMethod::ProveCommitSectors3 as u64,
            params: Some(prove_commit_params_ser),
            subinvocs: Some(vec![
                Expect::reward_this_epoch(miner_id),
                Expect::power_current_total(miner_id),
                Expect::power_update_pledge(miner_id, None),
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

    // FIP-0118: Miner no longer calls verifreg for claim validation during extensions.

    let mut subinvocs = vec![];
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
    // FIP-0118: Market no longer does datacap ops for verified deals.
    // The verified_deal flag is kept for backward compat but is functionally ignored.
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

/// Spacetime a deal's piece occupies over the given duration.
pub fn deal_spacetime(v: &dyn VM, deal_id: DealID, duration: ChainEpoch) -> DealWeight {
    DealWeight::from(get_deal(v, deal_id).piece_size.0 * duration as u64)
}

pub fn make_piece_manifests_from_deal_ids(
    v: &dyn VM,
    deal_ids: Vec<DealID>,
) -> Vec<PieceActivationManifest> {
    let mut piece_manifests = vec![];
    for deal_id in deal_ids {
        let deal = get_deal(v, deal_id);
        // FIP-0118: Market no longer stores pending deal allocations.
        // verified_allocation_key is kept for API backward compat but is ignored by miner.

        piece_manifests.push(PieceActivationManifest {
            cid: deal.piece_cid,
            size: deal.piece_size,
            verified_allocation_key: None,
            notify: vec![DataActivationNotification {
                address: STORAGE_MARKET_ACTOR_ADDR,
                payload: serialize(&deal_id, "dealid").unwrap(),
            }],
        });
    }
    piece_manifests
}
