use std::ops::Neg;

use export_macro::vm_test;
use fvm_shared::bigint::Zero;
use fvm_shared::econ::TokenAmount;
use fvm_shared::piece::PaddedPieceSize;
use fvm_shared::sector::{RegisteredSealProof, SectorNumber, StoragePower};

use fil_actor_datacap::State as DatacapState;
use fil_actor_market::{DealArray, DealMetaArray, DealSettlementSummary};
use fil_actor_market::{
    PendingDealAllocationsMap, State as MarketState, PENDING_ALLOCATIONS_CONFIG,
};
use fil_actor_miner::{max_prove_commit_duration, PowerPair, SectorClaim, State as MinerState};
use fil_actor_power::State as PowerState;
use fil_actor_verifreg::{
    Claim, Method as VerifregMethod, RemoveExpiredClaimsParams, RemoveExpiredClaimsReturn,
    State as VerifregState,
};
use fil_actors_runtime::cbor::deserialize;
use fil_actors_runtime::runtime::policy_constants::{
    DEAL_UPDATES_INTERVAL, MARKET_DEFAULT_ALLOCATION_TERM_BUFFER,
};
use fil_actors_runtime::runtime::Policy;
use fil_actors_runtime::test_utils::make_piece_cid;
use fil_actors_runtime::{
    DealWeight, DATACAP_TOKEN_ACTOR_ADDR, EPOCHS_IN_DAY, STORAGE_MARKET_ACTOR_ADDR,
    STORAGE_POWER_ACTOR_ADDR, VERIFIED_REGISTRY_ACTOR_ADDR,
};
use vm_api::util::{apply_ok, get_state, DynBlockstore};
use vm_api::VM;

use crate::util::{
    advance_by_deadline_to_epoch, advance_by_deadline_to_epoch_while_proving,
    advance_by_deadline_to_index, advance_to_proving_deadline, assert_invariants, create_accounts,
    create_miner, cron_tick, datacap_extend_claim, datacap_get_balance, expect_invariants,
    invariant_failure_patterns, market_add_balance, market_pending_deal_allocations,
    market_publish_deal, miner_extend_sector_expiration2, miner_precommit_one_sector_v2,
    miner_prove_sector, precommit_meta_data_from_deals, provider_settle_deal_payments,
    sector_deadline, submit_windowed_post, verifreg_add_client, verifreg_add_verifier,
    verifreg_extend_claim_terms, verifreg_remove_expired_allocations,
};

/// Tests a scenario involving a verified deal from the built-in market, with associated
/// allocation and claim.
/// This test shares some set-up copied from extend_sectors_test.
#[vm_test]
pub fn verified_claim_scenario_test(v: &dyn VM) {
    let addrs = create_accounts(v, 4, &TokenAmount::from_whole(10_000));
    let seal_proof = RegisteredSealProof::StackedDRG32GiBV1P1;
    let (owner, worker, verifier, verified_client, verified_client2) =
        (addrs[0], addrs[0], addrs[1], addrs[2], addrs[3]);
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

    // Register verifier and verified clients
    let datacap = StoragePower::from(32_u128 << 40);
    verifreg_add_verifier(v, &verifier, &datacap * 2);
    verifreg_add_client(v, &verifier, &verified_client, datacap.clone());
    verifreg_add_client(v, &verifier, &verified_client2, datacap.clone());

    // Add market collateral for client and miner
    // Client2 doesn't need collateral because they won't make a new deal, only extend a claim.
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

    let claim_id = market_pending_deal_allocations(v, &deals)[0];

    // Precommit and prove the sector for the max term allowed by the deal.
    let sector_term = deal_term_min + MARKET_DEFAULT_ALLOCATION_TERM_BUFFER;
    let _precommit = miner_precommit_one_sector_v2(
        v,
        &worker,
        &miner_id,
        seal_proof,
        sector_number,
        precommit_meta_data_from_deals(v, &deals, seal_proof),
        true,
        deal_start + sector_term,
    );

    // Advance time to max seal duration and prove the sector
    advance_by_deadline_to_epoch(v, &miner_id, deal_start);
    miner_prove_sector(v, &worker, &miner_id, sector_number);
    // Trigger cron to validate the prove commit
    cron_tick(v);

    // Verify sector info
    let miner_state: MinerState = get_state(v, &miner_id).unwrap();
    let sector_info = miner_state
        .get_sector(&DynBlockstore::wrap(v.blockstore()), sector_number)
        .unwrap()
        .unwrap();
    assert_eq!(sector_term, sector_info.expiration - sector_info.activation);
    assert_eq!(DealWeight::zero(), sector_info.deal_weight);
    // Verified weight is sector term * 32 GiB, using simple QAP
    let verified_weight = DealWeight::from(sector_term as u64 * deal_size);
    assert_eq!(verified_weight, sector_info.verified_deal_weight);

    // Verify datacap state
    let datacap_state: DatacapState = get_state(v, &DATACAP_TOKEN_ACTOR_ADDR).unwrap();
    assert_eq!(
        TokenAmount::from_whole(datacap.clone()) - TokenAmount::from_whole(deal_size), // Spent deal size
        datacap_state
            .token
            .get_balance(&DynBlockstore::wrap(v.blockstore()), verified_client.id().unwrap())
            .unwrap()
    );
    assert_eq!(
        TokenAmount::from_whole(datacap.clone()), // Nothing spent
        datacap_state
            .token
            .get_balance(&DynBlockstore::wrap(v.blockstore()), verified_client2.id().unwrap())
            .unwrap()
    );
    assert_eq!(
        TokenAmount::zero(), // Burnt when the allocation was claimed
        datacap_state
            .token
            .get_balance(
                &DynBlockstore::wrap(v.blockstore()),
                VERIFIED_REGISTRY_ACTOR_ADDR.id().unwrap()
            )
            .unwrap()
    );
    assert_eq!(
        TokenAmount::from_whole(datacap.clone()) * 2 - TokenAmount::from_whole(deal_size),
        datacap_state.token.supply
    );

    // Verify claim state
    let verifreg_state: VerifregState = get_state(v, &VERIFIED_REGISTRY_ACTOR_ADDR).unwrap();
    let store = DynBlockstore::wrap(v.blockstore());
    let mut claims = verifreg_state.load_claims(&store).unwrap();
    let claim = claims.get(miner_id.id().unwrap(), claim_id).unwrap().unwrap();
    assert_eq!(sector_number, claim.sector);
    assert_eq!(
        &Claim {
            provider: miner_id.id().unwrap(),
            client: verified_client.id().unwrap(),
            data: make_piece_cid("deal1".as_bytes()),
            size: PaddedPieceSize(deal_size),
            term_min: deal_term_min,
            term_max: deal_term_min + 90 * EPOCHS_IN_DAY,
            term_start: deal_start,
            sector: sector_number,
        },
        claim
    );

    // Advance to proving period and submit post
    let (deadline_info, partition_index) = advance_to_proving_deadline(v, &miner_id, sector_number);

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

    // Advance past the deal's minimum term (the claim remains valid).
    advance_by_deadline_to_epoch_while_proving(
        v,
        &miner_id,
        &worker,
        sector_number,
        deal_start + deal_term_min + 10,
    );

    // The client extends the verified claim term out to 12 months.
    verifreg_extend_claim_terms(v, &verified_client, &miner_id, claim_id, 360 * EPOCHS_IN_DAY);

    // Now the miner can extend the sector's expiration to the same.
    let (didx, pidx) = sector_deadline(v, &miner_id, sector_number);
    let extended_expiration_1 = deal_start + 360 * EPOCHS_IN_DAY;
    miner_extend_sector_expiration2(
        v,
        &worker,
        &miner_id,
        didx,
        pidx,
        vec![],
        vec![SectorClaim { sector_number, maintain_claims: vec![claim_id], drop_claims: vec![] }],
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

    // Another client extends the claim beyond the initial maximum term.
    let original_max_term = policy.maximum_verified_allocation_term;
    let new_claim_expiry_epoch = v.epoch() + policy.maximum_verified_allocation_term;
    let new_max_term = new_claim_expiry_epoch - claim.term_start;
    assert!(new_max_term > original_max_term);

    datacap_extend_claim(
        v,
        &verified_client2,
        &miner_id,
        claim_id,
        deal_size,
        new_max_term,
        verified_client.id().unwrap(),
    );

    // The miner extends the sector into the second year.
    let extended_expiration_2 = extended_expiration_1 + 60 * EPOCHS_IN_DAY;
    miner_extend_sector_expiration2(
        v,
        &worker,
        &miner_id,
        didx,
        pidx,
        vec![],
        vec![SectorClaim { sector_number, maintain_claims: vec![claim_id], drop_claims: vec![] }],
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

    // The miner can drop the claim, losing the multiplied QA power.
    let expected_power_delta =
        PowerPair::new(StoragePower::zero(), StoragePower::from(9 * deal_size).neg());
    miner_extend_sector_expiration2(
        v,
        &worker,
        &miner_id,
        didx,
        pidx,
        vec![],
        vec![SectorClaim { sector_number, maintain_claims: vec![], drop_claims: vec![claim_id] }],
        extended_expiration_2, // No change in expiration
        expected_power_delta,  // Power lost
    );

    // Verify sector info
    let miner_state: MinerState = get_state(v, &miner_id).unwrap();
    let sector_info = miner_state
        .get_sector(&DynBlockstore::wrap(v.blockstore()), sector_number)
        .unwrap()
        .unwrap();
    assert_eq!(extended_expiration_2, sector_info.expiration);
    assert_eq!(DealWeight::zero(), sector_info.deal_weight);
    assert_eq!(DealWeight::zero(), sector_info.verified_deal_weight);
    // No longer verified

    // Verify datacap state
    let datacap_state: DatacapState = get_state(v, &DATACAP_TOKEN_ACTOR_ADDR).unwrap();
    assert_eq!(
        TokenAmount::from_whole(datacap.clone()) - TokenAmount::from_whole(deal_size), // Spent deal size
        datacap_state
            .token
            .get_balance(&DynBlockstore::wrap(v.blockstore()), verified_client.id().unwrap())
            .unwrap()
    );
    assert_eq!(
        TokenAmount::from_whole(datacap.clone()) - TokenAmount::from_whole(deal_size), // Also spent deal size
        datacap_state
            .token
            .get_balance(&DynBlockstore::wrap(v.blockstore()), verified_client2.id().unwrap())
            .unwrap()
    );
    assert_eq!(
        TokenAmount::zero(), // All burnt
        datacap_state
            .token
            .get_balance(
                &DynBlockstore::wrap(v.blockstore()),
                VERIFIED_REGISTRY_ACTOR_ADDR.id().unwrap()
            )
            .unwrap()
    );
    assert_eq!(
        TokenAmount::from_whole(datacap) * 2 - TokenAmount::from_whole(deal_size) * 2, // Spent deal size twice
        datacap_state.token.supply
    );

    // Advance sector to expiration
    advance_by_deadline_to_epoch_while_proving(
        v,
        &miner_id,
        &worker,
        sector_number,
        extended_expiration_2,
    );
    // And advance vm past the claim's max term (no more sector exists to prove)
    v.set_epoch(new_claim_expiry_epoch);
    // Expired claim can now be cleaned up
    let cleanup_claims =
        RemoveExpiredClaimsParams { provider: miner_id.id().unwrap(), claim_ids: vec![claim_id] };

    let ret_raw = apply_ok(
        v,
        &worker,
        &VERIFIED_REGISTRY_ACTOR_ADDR,
        &TokenAmount::zero(),
        VerifregMethod::RemoveExpiredClaims as u64,
        Some(cleanup_claims),
    );
    let ret: RemoveExpiredClaimsReturn = deserialize(&ret_raw, "balance of return value").unwrap();
    assert_eq!(vec![claim_id], ret.considered);
    assert!(ret.results.all_ok(), "results had failures {}", ret.results);

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

#[vm_test]
pub fn expired_allocations_test(v: &dyn VM) {
    let addrs = create_accounts(v, 3, &TokenAmount::from_whole(10_000));
    let seal_proof = RegisteredSealProof::StackedDRG32GiBV1P1;
    let (owner, worker, verifier, verified_client) = (addrs[0], addrs[0], addrs[1], addrs[2]);

    // Create miner
    let (miner_id, _) = create_miner(
        v,
        &owner,
        &worker,
        seal_proof.registered_window_post_proof().unwrap(),
        &TokenAmount::from_whole(1_000),
    );
    v.set_epoch(200);

    // Register verifier and verified clients
    let datacap = StoragePower::from(32_u128 << 40);
    verifreg_add_verifier(v, &verifier, &datacap * 2);
    verifreg_add_client(v, &verifier, &verified_client, datacap.clone());

    // Add market collateral for client and miner
    market_add_balance(v, &verified_client, &verified_client, &TokenAmount::from_whole(3));
    market_add_balance(v, &worker, &miner_id, &TokenAmount::from_whole(64));

    // Publish 2 verified deals
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

    // Client datacap balance reduced
    assert_eq!(
        TokenAmount::from_whole(datacap.clone()) - TokenAmount::from_whole(deal_size),
        datacap_get_balance(v, &verified_client)
    );

    // Advance to after the first deal's start
    v.set_epoch(deal1_start + DEAL_UPDATES_INTERVAL);
    cron_tick(v);

    // Deal has expired and cleaned up.
    let market_state: MarketState = get_state(v, &STORAGE_MARKET_ACTOR_ADDR).unwrap();
    let store = DynBlockstore::wrap(v.blockstore());
    let proposals = DealArray::load(&market_state.proposals, &store).unwrap();
    assert!(proposals.get(deal1).unwrap().is_none());
    let pending_deal_allocs = PendingDealAllocationsMap::load(
        &store,
        &market_state.pending_deal_allocation_ids,
        PENDING_ALLOCATIONS_CONFIG,
        "pending allocations",
    )
    .unwrap();
    assert!(pending_deal_allocs.get(&deal1).unwrap().is_none());

    // Allocation still exists until explicit cleanup
    let alloc_id = 1;
    let verifreg_state: VerifregState = get_state(v, &VERIFIED_REGISTRY_ACTOR_ADDR).unwrap();
    let store = DynBlockstore::wrap(v.blockstore());
    let mut allocs = verifreg_state.load_allocs(&store).unwrap();
    assert!(allocs.get(verified_client.id().unwrap(), alloc_id).unwrap().is_some());

    verifreg_remove_expired_allocations(
        v,
        &worker,
        &verified_client,
        vec![],
        deal_size,
        vec![alloc_id],
    );

    // Allocation is gone
    let verifreg_state: VerifregState = get_state(v, &VERIFIED_REGISTRY_ACTOR_ADDR).unwrap();
    let store = DynBlockstore::wrap(v.blockstore());
    let mut allocs = verifreg_state.load_allocs(&store).unwrap();
    assert!(allocs.get(verified_client.id().unwrap(), alloc_id).unwrap().is_none());

    // Client has original datacap balance
    assert_eq!(TokenAmount::from_whole(datacap), datacap_get_balance(v, &verified_client));

    expect_invariants(
        v,
        &Policy::default(),
        &[invariant_failure_patterns::REWARD_STATE_EPOCH_MISMATCH.to_owned()],
        None,
    );
}

#[vm_test]
pub fn deal_passes_claim_fails_test(v: &dyn VM) {
    let addrs = create_accounts(v, 3, &TokenAmount::from_whole(10_000));
    let seal_proof = RegisteredSealProof::StackedDRG32GiBV1P1;
    let (owner, worker, verifier, verified_client) = (addrs[0], addrs[0], addrs[1], addrs[2]);

    // Create miner
    let (miner_id, _) = create_miner(
        v,
        &owner,
        &worker,
        seal_proof.registered_window_post_proof().unwrap(),
        &TokenAmount::from_whole(1_000),
    );
    v.set_epoch(200);

    // Register verifier and verified clients
    let datacap = StoragePower::from(32_u128 << 40);
    verifreg_add_verifier(v, &verifier, &datacap * 2);
    verifreg_add_client(v, &verifier, &verified_client, datacap.clone());

    // Add market collateral for client and miner
    market_add_balance(v, &verified_client, &verified_client, &TokenAmount::from_whole(3));
    market_add_balance(v, &worker, &miner_id, &TokenAmount::from_whole(64));

    // Publish verified deal
    let deal_start = v.epoch() + Policy::default().maximum_verified_allocation_expiration + 1;
    let sector_start = deal_start;
    let deal_term_min = 180 * EPOCHS_IN_DAY;
    let deal_size = (32u64 << 30) / 2;
    // Deal is published so far in advance of prove commit that allocation will expire epoch before sector is committed
    let bad_deal = market_publish_deal(
        v,
        &worker,
        &verified_client,
        &miner_id,
        "baddeal".to_string(),
        PaddedPieceSize(deal_size),
        true,
        deal_start,
        deal_term_min,
    )
    .ids[0];
    // good deal is published 1 epoch later so that allocation will not expire
    advance_by_deadline_to_epoch(v, &miner_id, v.epoch() + 1);
    let deal = market_publish_deal(
        v,
        &worker,
        &verified_client,
        &miner_id,
        "deal".to_string(),
        PaddedPieceSize(deal_size),
        true,
        deal_start,
        deal_term_min,
    )
    .ids[0];

    // Client datacap balance reduced
    assert_eq!(
        TokenAmount::from_whole(datacap) - TokenAmount::from_whole(2 * deal_size),
        datacap_get_balance(v, &verified_client)
    );

    // Precommit and prove two sectors for the max term allowed by the deal.
    // First sector claims a deal with unexpired allocation
    // Second sector claims a deal with expired allocation
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
        precommit_meta_data_from_deals(v, &[deal], seal_proof),
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
        precommit_meta_data_from_deals(v, &[bad_deal], seal_proof),
        false,
        sector_start + sector_term,
    );

    // Advance time and prove the sector
    advance_by_deadline_to_epoch(v, &miner_id, sector_start);
    miner_prove_sector(v, &worker, &miner_id, sector_number_a);
    miner_prove_sector(v, &worker, &miner_id, sector_number_b);
    cron_tick(v);
    v.set_epoch(v.epoch() + 1);

    // check that deal is not activated

    // Verify deal state.
    let market_state: MarketState = get_state(v, &STORAGE_MARKET_ACTOR_ADDR).unwrap();
    let store = DynBlockstore::wrap(v.blockstore());
    let deal_states = DealMetaArray::load(&market_state.states, &store).unwrap();
    // bad deal sector can't be confirmed for commit so bad deal must not be included
    let bad_deal_state = deal_states.get(bad_deal).unwrap();
    assert_eq!(None, bad_deal_state);
    // deal sector fails because confirm for commit is now all or nothing
    let deal_state = deal_states.get(deal).unwrap();
    assert_eq!(None, deal_state);

    // Verify sector info
    let miner_state: MinerState = get_state(v, &miner_id).unwrap();
    // bad deal sector can't be confirmed for commit because alloc can't be claimed
    let sector_info_b =
        miner_state.get_sector(&DynBlockstore::wrap(v.blockstore()), sector_number_b).unwrap();
    assert_eq!(None, sector_info_b);
    // deal sector fails because confirm for commit is now all or nothing
    let sector_info_a =
        miner_state.get_sector(&DynBlockstore::wrap(v.blockstore()), sector_number_a).unwrap();
    assert_eq!(None, sector_info_a);

    // run check before last change and confirm that we hit the expected broken state error
    assert_invariants(v, &Policy::default(), None);
}
