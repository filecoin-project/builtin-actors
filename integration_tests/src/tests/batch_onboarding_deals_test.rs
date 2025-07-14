use fil_actor_market::DealProposal;
use fil_actor_miner::Method as MinerMethod;
use fil_actor_miner::{
    CompactCommD, DataActivationNotification, PieceActivationManifest, ProveCommitSectors3Params,
    SectorActivationManifest, SectorPreCommitOnChainInfo, State as MinerState,
    VerifiedAllocationKey, max_prove_commit_duration, power_for_sector,
};
use fil_actors_runtime::cbor::serialize;
use fil_actors_runtime::runtime::Policy;
use fil_actors_runtime::runtime::policy::policy_constants::PRE_COMMIT_CHALLENGE_DELAY;
use fil_actors_runtime::test_utils::make_piece_cid;
use fvm_ipld_encoding::RawBytes;
use fvm_shared::address::Address;
use fvm_shared::bigint::BigInt;
use fvm_shared::clock::ChainEpoch;
use fvm_shared::deal::DealID;
use fvm_shared::econ::TokenAmount;
use fvm_shared::error::ExitCode;
use fvm_shared::piece::PaddedPieceSize;
use fvm_shared::sector::{RegisteredAggregateProof, RegisteredSealProof, StoragePower};
use num_traits::Zero;

use export_macro::vm_test;
use fil_actor_miner::ext::verifreg::AllocationID;
use vm_api::VM;
use vm_api::util::{DynBlockstore, apply_ok, get_state};

use crate::deals::{DealBatcher, DealOptions};
use crate::util::{
    PrecommitMetadata, advance_to_proving_deadline, bf_all, create_accounts, create_miner,
    get_network_stats, market_add_balance, market_pending_deal_allocations, miner_balance,
    precommit_meta_data_from_deals, precommit_sectors_v2, precommit_sectors_v2_expect_code,
    submit_windowed_post, verifreg_add_client, verifreg_add_verifier,
};
use fil_actors_runtime::STORAGE_MARKET_ACTOR_ADDR;

const BATCH_SIZE: usize = 8;
const SEAL_PROOF: RegisteredSealProof = RegisteredSealProof::StackedDRG32GiBV1P1;

#[vm_test]
pub fn pre_commit_requires_commd_test(v: &dyn VM) {
    let deal_duration: ChainEpoch = Policy::default().min_sector_expiration;
    let sector_duration: ChainEpoch =
        deal_duration + Policy::default().market_default_allocation_term_buffer;

    let addrs = create_accounts(v, 2, &TokenAmount::from_whole(10_000));
    let (owner, client) = (addrs[0], addrs[1]);
    let worker = owner;

    // Create miner
    let (miner, _) = create_miner(
        v,
        &owner,
        &worker,
        SEAL_PROOF.registered_window_post_proof().unwrap(),
        &TokenAmount::from_whole(1000),
    );

    // Fund storage market accounts.
    market_add_balance(v, &owner, &miner, &TokenAmount::from_whole(1000));
    market_add_balance(v, &client, &client, &TokenAmount::from_whole(1000));

    // Publish a deal for the sector.
    let deal_opts = DealOptions {
        piece_size: PaddedPieceSize(32 * (1 << 30)),
        verified: false,
        deal_start: v.epoch() + max_prove_commit_duration(&Policy::default(), SEAL_PROOF).unwrap(),
        deal_lifetime: deal_duration,
        ..DealOptions::default()
    };
    let mut batcher = DealBatcher::new(v, deal_opts);
    batcher.stage(client, miner);
    let ret = batcher.publish_ok(worker);
    let good_inputs = bf_all(ret.valid_deals);
    assert_eq!(vec![0], good_inputs);

    // precommit without specifying commD fails
    let sector_number = 100;
    precommit_sectors_v2_expect_code(
        v,
        1,
        vec![PrecommitMetadata { deals: vec![0], commd: CompactCommD(None) }],
        &worker,
        &miner,
        SEAL_PROOF,
        sector_number,
        true,
        Some(sector_duration),
        ExitCode::USR_ILLEGAL_ARGUMENT,
    );

    // precommit specifying the wrong commD fails
    precommit_sectors_v2_expect_code(
        v,
        1,
        vec![PrecommitMetadata {
            deals: vec![0],
            commd: CompactCommD(Some(make_piece_cid("This is not commP".as_bytes()))),
        }],
        &worker,
        &miner,
        SEAL_PROOF,
        sector_number,
        true,
        Some(sector_duration),
        ExitCode::USR_ILLEGAL_ARGUMENT,
    );
}

// Tests batch onboarding of sectors with verified deals.
#[vm_test(1)]
pub fn batch_onboarding_deals_test(v: &dyn VM) {
    let deal_duration: ChainEpoch = Policy::default().min_sector_expiration;
    let sector_duration: ChainEpoch =
        deal_duration + Policy::default().market_default_allocation_term_buffer;

    let addrs = create_accounts(v, 3, &TokenAmount::from_whole(10_000));
    let (owner, verifier, client) = (addrs[0], addrs[1], addrs[2]);
    let worker = owner;

    // Create miner
    let (miner, _) = create_miner(
        v,
        &owner,
        &worker,
        SEAL_PROOF.registered_window_post_proof().unwrap(),
        &TokenAmount::from_whole(1000),
    );

    // Create FIL verifier and client.
    verifreg_add_verifier(v, &verifier, StoragePower::from((1000_u64 << 30) as u128));
    verifreg_add_client(v, &verifier, &client, StoragePower::from((1000_u64 << 30) as u128));

    // Fund storage market accounts.
    market_add_balance(v, &owner, &miner, &TokenAmount::from_whole(1000));
    market_add_balance(v, &client, &client, &TokenAmount::from_whole(1000));

    // Publish a deal for each sector.
    let deals = publish_deals(v, client, miner, worker, deal_duration, BATCH_SIZE);
    assert_eq!(BATCH_SIZE, deals.len());

    // Verify datacap allocations.
    let deal_keys: Vec<DealID> = deals.iter().map(|(id, _)| *id).collect();
    let alloc_ids = market_pending_deal_allocations(v, &deal_keys);
    assert_eq!(BATCH_SIZE, alloc_ids.len());

    // Associate deals with sectors, but don't include deal IDs in the pre-commit itself.
    let sector_precommit_data: Vec<PrecommitMetadata> = deals
        .iter()
        .map(|(id, _)| precommit_meta_data_from_deals(v, &[*id], SEAL_PROOF, false))
        .collect();

    // Pre-commit as a single batch.
    let precommits = precommit_sectors_v2(
        v,
        BATCH_SIZE,
        sector_precommit_data,
        &worker,
        &miner,
        SEAL_PROOF,
        0,
        true,
        Some(sector_duration),
    );
    let first_sector_no = precommits[0].info.sector_number;

    // Prove-commit as a single aggregate.
    v.set_epoch(v.epoch() + PRE_COMMIT_CHALLENGE_DELAY + 1);
    prove_commit_aggregate(v, &worker, deals, alloc_ids, &miner, &client, precommits);
    // Submit Window PoST to activate power.
    let (dline_info, p_idx) = advance_to_proving_deadline(v, &miner, 0);

    let sector_size = SEAL_PROOF.sector_size().unwrap();
    let st: MinerState = get_state(v, &miner).unwrap();
    let sector =
        st.get_sector(&DynBlockstore::wrap(v.blockstore()), first_sector_no).unwrap().unwrap();
    let mut expect_new_power = power_for_sector(sector_size, &sector);
    // Confirm the verified deal resulted in QA power.
    assert_eq!(&expect_new_power.raw * 10, expect_new_power.qa);
    expect_new_power.raw *= BATCH_SIZE;
    expect_new_power.qa *= BATCH_SIZE;
    submit_windowed_post(v, &worker, &miner, dline_info, p_idx, Some(expect_new_power.clone()));

    // Verify state expectations.
    let balances = miner_balance(v, &miner);
    assert!(balances.initial_pledge.is_positive());

    let network_stats = get_network_stats(v);
    assert_eq!(
        network_stats.total_bytes_committed,
        BigInt::from(sector_size as usize * BATCH_SIZE)
    );
    assert_eq!(network_stats.total_qa_bytes_committed, network_stats.total_bytes_committed * 10);
    assert!(network_stats.total_pledge_collateral.is_positive());
}

fn publish_deals(
    v: &dyn VM,
    client: Address,
    provider: Address,
    worker: Address,
    duration: ChainEpoch,
    count: usize,
) -> Vec<(DealID, DealProposal)> {
    let deal_opts = DealOptions {
        piece_size: PaddedPieceSize(32 * (1 << 30)),
        verified: true,
        deal_start: v.epoch() + max_prove_commit_duration(&Policy::default(), SEAL_PROOF).unwrap(),
        deal_lifetime: duration,
        ..DealOptions::default()
    };
    let mut batcher = DealBatcher::new(v, deal_opts);
    (0..count).for_each(|_| batcher.stage(client, provider));
    let ret = batcher.publish_ok(worker);
    let good_inputs = bf_all(ret.valid_deals);
    assert_eq!((0..count as u64).collect::<Vec<u64>>(), good_inputs);
    ret.ids.into_iter().zip(batcher.proposals().iter().cloned()).collect()
}

// This method doesn't check any trace expectations.
// We can do so by unifying with util::prove_commit_sectors, and plumbing through
// the information necessary to check expectations of deal activation and FIL+ claims.
// https://github.com/filecoin-project/builtin-actors/issues/1302
pub fn prove_commit_aggregate(
    v: &dyn VM,
    worker: &Address,
    deals: Vec<(DealID, DealProposal)>,
    alloc_ids: Vec<AllocationID>,
    miner: &Address,
    client: &Address,
    precommits: Vec<SectorPreCommitOnChainInfo>,
) {
    let client_id = client.id().unwrap();
    let sector_activations: Vec<SectorActivationManifest> = precommits
        .iter()
        .zip(deals.iter())
        .zip(alloc_ids.iter())
        .map(|((pc, (deal_id, deal_proposal)), alloc_id)| SectorActivationManifest {
            sector_number: pc.info.sector_number,
            pieces: vec![PieceActivationManifest {
                cid: deal_proposal.piece_cid,
                size: deal_proposal.piece_size,
                verified_allocation_key: Some(VerifiedAllocationKey {
                    client: client_id,
                    id: *alloc_id,
                }),
                notify: vec![DataActivationNotification {
                    address: STORAGE_MARKET_ACTOR_ADDR,
                    payload: serialize(deal_id, "deal id").unwrap(),
                }],
            }],
        })
        .collect();

    let params = ProveCommitSectors3Params {
        sector_activations,
        sector_proofs: vec![],
        aggregate_proof: RawBytes::new(vec![0; 192]),
        aggregate_proof_type: Some(RegisteredAggregateProof::SnarkPackV2),
        require_activation_success: true,
        require_notification_success: true,
    };

    apply_ok(
        v,
        &worker,
        &miner,
        &TokenAmount::zero(),
        MinerMethod::ProveCommitSectors3 as u64,
        Some(params),
    );
}
