use export_macro::vm_test;
use fvm_ipld_encoding::RawBytes;
use fvm_shared::bigint::Zero;
use fvm_shared::econ::TokenAmount;
use fvm_shared::piece::PaddedPieceSize;
use fvm_shared::sector::{RegisteredSealProof, SectorNumber, StoragePower};

use fil_actor_market::State as MarketState;
use fil_actor_market::{DealArray, DealSettlementSummary};
use fil_actor_miner::{
    PowerPair, ProveCommitSectors3Params, SectorActivationManifest, State as MinerState,
    max_prove_commit_duration,
};
use fil_actor_power::State as PowerState;
use fil_actors_runtime::runtime::Policy;
use fil_actors_runtime::runtime::policy_constants::{
    DEAL_UPDATES_INTERVAL, MARKET_DEFAULT_ALLOCATION_TERM_BUFFER,
};
use fil_actors_runtime::{EPOCHS_IN_DAY, STORAGE_MARKET_ACTOR_ADDR, STORAGE_POWER_ACTOR_ADDR};
use vm_api::VM;
use vm_api::util::{DynBlockstore, apply_ok, get_state};

use crate::util::{
    advance_by_deadline_to_epoch, advance_by_deadline_to_epoch_while_proving,
    advance_by_deadline_to_index, advance_to_proving_deadline, assert_invariants, create_accounts,
    create_miner, cron_tick, expect_invariants, invariant_failure_patterns,
    make_piece_manifests_from_deal_ids, market_add_balance, market_publish_deal,
    miner_extend_sector_expiration2, miner_precommit_one_sector_v2, miner_prove_sector,
    precommit_meta_data_from_deals, provider_settle_deal_payments, sector_deadline,
    submit_windowed_post,
};

/// FIP-0118: Tests a scenario involving a deal from the built-in market.
/// With FIP-0118, verified deals no longer create allocations or claims.
/// All sectors get 10x QA power regardless of verified status.
#[vm_test]
pub fn verified_claim_scenario_test(v: &dyn VM) {
    let addrs = create_accounts(v, 4, &TokenAmount::from_whole(10_000));
    let seal_proof = RegisteredSealProof::StackedDRG32GiBV1P1;
    let (owner, worker, _verifier, verified_client) = (addrs[0], addrs[0], addrs[1], addrs[2]);
    let sector_number: SectorNumber = 100;
    let policy = Policy::default();

    // Create miner
    let (miner_id, _) = create_miner(
        v,
        &owner,
        &worker,
        seal_proof.registered_window_post_proof().unwrap(),
        &TokenAmount::from_whole(1_000),
    );
    v.set_epoch(200);

    // FIP-0118: No need to set up verifier/verified client (minting deprecated)

    // Add market collateral for client and miner
    market_add_balance(v, &verified_client, &verified_client, &TokenAmount::from_whole(3));
    market_add_balance(v, &worker, &miner_id, &TokenAmount::from_whole(64));

    // Publish a verified deal for total sector capacity with min term of 6 months
    let deal_start = v.epoch() + max_prove_commit_duration(&Policy::default(), seal_proof).unwrap();
    let deal_term_min = 180 * EPOCHS_IN_DAY;

    let deal_size = 32u64 << 30;
    let deals = market_publish_deal(
        v,
        &worker,
        &verified_client,
        &miner_id,
        "deal1".to_string(),
        PaddedPieceSize(deal_size),
        true,
        deal_start,
        deal_term_min,
    )
    .ids;

    // Precommit and prove the sector
    let sector_term = deal_term_min + MARKET_DEFAULT_ALLOCATION_TERM_BUFFER;
    let _precommit = miner_precommit_one_sector_v2(
        v,
        &worker,
        &miner_id,
        seal_proof,
        sector_number,
        precommit_meta_data_from_deals(v, &deals, seal_proof, false),
        true,
        deal_start + sector_term,
    );

    // Advance time to max seal duration and prove the sector
    advance_by_deadline_to_epoch(v, &miner_id, deal_start);
    miner_prove_sector(
        v,
        &worker,
        &miner_id,
        sector_number,
        make_piece_manifests_from_deal_ids(v, deals.clone()),
    );
    // Trigger cron to validate the prove commit
    cron_tick(v);

    // Verify sector info
    let miner_state: MinerState = get_state(v, &miner_id).unwrap();
    let sector_info = miner_state
        .get_sector(&DynBlockstore::wrap(v.blockstore()), sector_number)
        .unwrap()
        .unwrap();
    assert_eq!(sector_term, sector_info.expiration - sector_info.activation);

    // Advance to proving period and submit post
    let (deadline_info, partition_index) = advance_to_proving_deadline(v, &miner_id, sector_number);

    // FIP-0118: All sectors get 10x QA power
    let expected_power =
        PowerPair { raw: StoragePower::from(deal_size), qa: StoragePower::from(10 * deal_size) };
    submit_windowed_post(
        v,
        &worker,
        &miner_id,
        deadline_info,
        partition_index,
        Some(expected_power.clone()),
    );

    // Verify miner power
    let power_state: PowerState = get_state(v, &STORAGE_POWER_ACTOR_ADDR).unwrap();
    let power_claim =
        power_state.get_claim(&DynBlockstore::wrap(v.blockstore()), &miner_id).unwrap().unwrap();
    assert_eq!(power_claim.raw_byte_power, expected_power.raw);
    assert_eq!(power_claim.quality_adj_power, expected_power.qa);

    // move forward one deadline so advanceWhileProving doesn't fail double submitting posts.
    advance_by_deadline_to_index(
        v,
        &miner_id,
        deadline_info.index + 1 % policy.wpost_period_deadlines,
    );

    // Advance past the deal's minimum term
    advance_by_deadline_to_epoch_while_proving(
        v,
        &miner_id,
        &worker,
        sector_number,
        deal_start + deal_term_min + 10,
    );

    // FIP-0118: No claim extensions needed. Extensions work without claim validation.
    let (didx, pidx) = sector_deadline(v, &miner_id, sector_number);
    let extended_expiration_1 = deal_start + 360 * EPOCHS_IN_DAY;
    miner_extend_sector_expiration2(
        v,
        &worker,
        &miner_id,
        didx,
        pidx,
        vec![sector_number],
        vec![],
        extended_expiration_1,
        PowerPair::zero(), // No change in power
    );

    // Advance toward the sector's expiration
    advance_by_deadline_to_epoch_while_proving(
        v,
        &miner_id,
        &worker,
        sector_number,
        extended_expiration_1 - 100,
    );

    // Extend again
    let extended_expiration_2 = extended_expiration_1 + 60 * EPOCHS_IN_DAY;
    miner_extend_sector_expiration2(
        v,
        &worker,
        &miner_id,
        didx,
        pidx,
        vec![sector_number],
        vec![],
        extended_expiration_2,
        PowerPair::zero(), // No change in power
    );

    // Advance toward the sector's new expiration
    advance_by_deadline_to_epoch_while_proving(
        v,
        &miner_id,
        &worker,
        sector_number,
        extended_expiration_2 - 30 * EPOCHS_IN_DAY,
    );

    // Verify sector info
    let miner_state: MinerState = get_state(v, &miner_id).unwrap();
    let sector_info = miner_state
        .get_sector(&DynBlockstore::wrap(v.blockstore()), sector_number)
        .unwrap()
        .unwrap();
    assert_eq!(extended_expiration_2, sector_info.expiration);

    let market_state: MarketState = get_state(v, &STORAGE_MARKET_ACTOR_ADDR).unwrap();
    let store = DynBlockstore::wrap(v.blockstore());
    let proposals = DealArray::load(&market_state.proposals, &store).unwrap();
    let proposal = proposals.get(deals[0]).unwrap().unwrap();
    // provider must process the deals to receive payment and cleanup state
    let ret = provider_settle_deal_payments(v, &miner_id, &deals);
    assert_eq!(
        ret.settlements.first().unwrap(),
        &DealSettlementSummary { payment: proposal.total_storage_fee(), completed: true }
    );

    expect_invariants(
        v,
        &Policy::default(),
        &[invariant_failure_patterns::REWARD_STATE_EPOCH_MISMATCH.to_owned()],
        None,
    );
}

/// FIP-0118: Market no longer creates allocations for verified deals.
/// This test verifies that deals expire correctly without allocation tracking.
#[vm_test]
pub fn expired_allocations_test(v: &dyn VM) {
    let addrs = create_accounts(v, 3, &TokenAmount::from_whole(10_000));
    let seal_proof = RegisteredSealProof::StackedDRG32GiBV1P1;
    let (owner, worker, _verifier, verified_client) = (addrs[0], addrs[0], addrs[1], addrs[2]);

    // Create miner
    let (miner_id, _) = create_miner(
        v,
        &owner,
        &worker,
        seal_proof.registered_window_post_proof().unwrap(),
        &TokenAmount::from_whole(1_000),
    );
    v.set_epoch(200);

    // FIP-0118: No verifreg setup needed

    // Add market collateral for client and miner
    market_add_balance(v, &verified_client, &verified_client, &TokenAmount::from_whole(3));
    market_add_balance(v, &worker, &miner_id, &TokenAmount::from_whole(64));

    // Publish verified deal
    let deal1_start =
        v.epoch() + max_prove_commit_duration(&Policy::default(), seal_proof).unwrap();
    let deal_term_min = 180 * EPOCHS_IN_DAY;

    let deal_size = 32u64 << 30;
    let deal1 = market_publish_deal(
        v,
        &worker,
        &verified_client,
        &miner_id,
        "deal1".to_string(),
        PaddedPieceSize(deal_size),
        true,
        deal1_start,
        deal_term_min,
    )
    .ids[0];

    // Advance to after the first deal's start
    v.set_epoch(deal1_start + DEAL_UPDATES_INTERVAL);
    cron_tick(v);

    // Deal has expired and cleaned up.
    let market_state: MarketState = get_state(v, &STORAGE_MARKET_ACTOR_ADDR).unwrap();
    let store = DynBlockstore::wrap(v.blockstore());
    let proposals = DealArray::load(&market_state.proposals, &store).unwrap();
    assert!(proposals.get(deal1).unwrap().is_none());

    expect_invariants(
        v,
        &Policy::default(),
        &[invariant_failure_patterns::REWARD_STATE_EPOCH_MISMATCH.to_owned()],
        None,
    );
}

/// FIP-0118: Market no longer creates allocations, so claim failures don't occur.
/// This test verifies that deals can be committed successfully without claim validation.
#[vm_test]
pub fn deal_passes_claim_fails_test(v: &dyn VM) {
    let addrs = create_accounts(v, 3, &TokenAmount::from_whole(10_000));
    let seal_proof = RegisteredSealProof::StackedDRG32GiBV1P1;
    let (owner, worker, _verifier, verified_client) = (addrs[0], addrs[0], addrs[1], addrs[2]);

    // Create miner
    let (miner_id, _) = create_miner(
        v,
        &owner,
        &worker,
        seal_proof.registered_window_post_proof().unwrap(),
        &TokenAmount::from_whole(1_000),
    );
    v.set_epoch(200);

    // FIP-0118: No verifreg setup needed

    // Add market collateral for client and miner
    market_add_balance(v, &verified_client, &verified_client, &TokenAmount::from_whole(3));
    market_add_balance(v, &worker, &miner_id, &TokenAmount::from_whole(64));

    // Publish verified deals
    let deal_start = v.epoch() + Policy::default().maximum_verified_allocation_expiration + 1;
    let sector_start = deal_start;
    let deal_term_min = 180 * EPOCHS_IN_DAY;
    let deal_size = (32u64 << 30) / 2;
    let deal1 = market_publish_deal(
        v,
        &worker,
        &verified_client,
        &miner_id,
        "deal1".to_string(),
        PaddedPieceSize(deal_size),
        true,
        deal_start,
        deal_term_min,
    )
    .ids[0];
    advance_by_deadline_to_epoch(v, &miner_id, v.epoch() + 1);
    let deal2 = market_publish_deal(
        v,
        &worker,
        &verified_client,
        &miner_id,
        "deal2".to_string(),
        PaddedPieceSize(deal_size),
        true,
        deal_start,
        deal_term_min,
    )
    .ids[0];

    // Precommit and prove two sectors
    let sector_term = deal_term_min + MARKET_DEFAULT_ALLOCATION_TERM_BUFFER;
    advance_by_deadline_to_epoch(
        v,
        &miner_id,
        sector_start - max_prove_commit_duration(&Policy::default(), seal_proof).unwrap(),
    );
    let sector_number_a = 0;
    let _precommit = miner_precommit_one_sector_v2(
        v,
        &worker,
        &miner_id,
        seal_proof,
        sector_number_a,
        precommit_meta_data_from_deals(v, &[deal2], seal_proof, false),
        true,
        sector_start + sector_term,
    );
    let sector_number_b = 1;
    let _precommit = miner_precommit_one_sector_v2(
        v,
        &worker,
        &miner_id,
        seal_proof,
        sector_number_b,
        precommit_meta_data_from_deals(v, &[deal1], seal_proof, false),
        false,
        sector_start + sector_term,
    );

    // FIP-0118: Without claim validation, both sectors succeed
    advance_by_deadline_to_epoch(v, &miner_id, sector_start);
    let prove_commit_params = ProveCommitSectors3Params {
        sector_activations: vec![
            SectorActivationManifest {
                sector_number: sector_number_b,
                pieces: make_piece_manifests_from_deal_ids(v, vec![deal1]),
            },
            SectorActivationManifest {
                sector_number: sector_number_a,
                pieces: make_piece_manifests_from_deal_ids(v, vec![deal2]),
            },
        ],
        sector_proofs: vec![vec![].into(), vec![].into()],
        aggregate_proof: RawBytes::default(),
        aggregate_proof_type: None,
        require_activation_success: true,
        require_notification_success: true,
    };
    apply_ok(
        v,
        &worker,
        &miner_id,
        &TokenAmount::zero(),
        fil_actor_miner::Method::ProveCommitSectors3 as u64,
        Some(prove_commit_params),
    );

    cron_tick(v);
    v.set_epoch(v.epoch() + 1);

    // Both sectors should be committed
    let miner_state: MinerState = get_state(v, &miner_id).unwrap();
    let sector_info_a =
        miner_state.get_sector(&DynBlockstore::wrap(v.blockstore()), sector_number_a).unwrap();
    assert!(sector_info_a.is_some(), "Sector A should have been committed");
    let sector_info_b =
        miner_state.get_sector(&DynBlockstore::wrap(v.blockstore()), sector_number_b).unwrap();
    assert!(sector_info_b.is_some(), "Sector B should have been committed");

    assert_invariants(v, &Policy::default(), None);
}
