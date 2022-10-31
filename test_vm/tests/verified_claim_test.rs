use fvm_ipld_blockstore::MemoryBlockstore;
use fvm_shared::bigint::Zero;
use fvm_shared::econ::TokenAmount;
use fvm_shared::piece::PaddedPieceSize;
use fvm_shared::sector::{RegisteredSealProof, SectorNumber, StoragePower};
use std::ops::Neg;

use fil_actor_datacap::State as DatacapState;
use fil_actor_market::State as MarketState;
use fil_actor_market::{deal_id_key, DealArray, DealMetaArray};
use fil_actor_miner::{max_prove_commit_duration, PowerPair, SectorClaim, State as MinerState};
use fil_actor_power::State as PowerState;
use fil_actor_verifreg::{
    AllocationID, Claim, Method as VerifregMethod, RemoveExpiredClaimsParams,
    RemoveExpiredClaimsReturn, State as VerifregState,
};
use fil_actors_runtime::cbor::deserialize;
use fil_actors_runtime::runtime::policy_constants::{
    DEAL_UPDATES_INTERVAL, MARKET_DEFAULT_ALLOCATION_TERM_BUFFER,
};
use fil_actors_runtime::runtime::Policy;
use fil_actors_runtime::shared::HAMT_BIT_WIDTH;
use fil_actors_runtime::test_utils::make_piece_cid;
use fil_actors_runtime::{
    make_map_with_root_and_bitwidth, DealWeight, Map, DATACAP_TOKEN_ACTOR_ADDR, EPOCHS_IN_DAY,
    STORAGE_MARKET_ACTOR_ADDR, STORAGE_POWER_ACTOR_ADDR, VERIFIED_REGISTRY_ACTOR_ADDR,
};

use test_vm::util::{
    advance_by_deadline_to_epoch, advance_by_deadline_to_epoch_while_proving,
    advance_by_deadline_to_index, advance_to_proving_deadline, apply_ok, create_accounts,
    create_miner, cron_tick, datacap_extend_claim, datacap_get_balance, invariant_failure_patterns,
    market_add_balance, market_publish_deal, miner_extend_sector_expiration2,
    miner_precommit_sector, miner_prove_sector, sector_deadline, submit_windowed_post,
    verifreg_add_client, verifreg_add_verifier, verifreg_extend_claim_terms,
    verifreg_remove_expired_allocations,
};
use test_vm::VM;

// Tests a scenario involving a verified deal from the built-in market, with associated
// allocation and claim.
// This test shares some set-up copied from extend_sectors_test.
#[test]
fn verified_claim_scenario() {
    let store = MemoryBlockstore::new();
    let mut v = VM::new_with_singletons(&store);
    let addrs = create_accounts(&v, 4, TokenAmount::from_whole(10_000));
    let seal_proof = RegisteredSealProof::StackedDRG32GiBV1P1;
    let (owner, worker, verifier, verified_client, verified_client2) =
        (addrs[0], addrs[0], addrs[1], addrs[2], addrs[3]);
    let sector_number: SectorNumber = 100;
    let policy = Policy::default();

    // Create miner
    let (miner_id, _) = create_miner(
        &mut v,
        owner,
        worker,
        seal_proof.registered_window_post_proof().unwrap(),
        TokenAmount::from_whole(1_000),
    );
    let mut v = v.with_epoch(200);

    // Register verifier and verified clients
    let datacap = StoragePower::from(32_u128 << 40);
    verifreg_add_verifier(&v, verifier, &datacap * 2);
    verifreg_add_client(&v, verifier, verified_client, datacap.clone());
    verifreg_add_client(&v, verifier, verified_client2, datacap.clone());

    // Add market collateral for client and miner
    // Client2 doesn't need collateral because they won't make a new deal, only extend a claim.
    market_add_balance(&v, verified_client, verified_client, TokenAmount::from_whole(3));
    market_add_balance(&v, worker, miner_id, TokenAmount::from_whole(64));

    // Publish a verified deal for total sector capacity with min term of 6 months
    let deal_start =
        v.get_epoch() + max_prove_commit_duration(&Policy::default(), seal_proof).unwrap();
    let deal_term_min = 180 * EPOCHS_IN_DAY;

    let deal_size = 32u64 << 30;
    let deals = market_publish_deal(
        &v,
        worker,
        verified_client,
        miner_id,
        "deal1".to_string(),
        PaddedPieceSize(deal_size),
        true,
        deal_start,
        deal_term_min,
    )
    .ids;

    // Precommit and prove the sector for the max term allowed by the deal.
    let sector_term = deal_term_min + MARKET_DEFAULT_ALLOCATION_TERM_BUFFER;
    let _precommit = miner_precommit_sector(
        &v,
        worker,
        miner_id,
        seal_proof,
        sector_number,
        deals.clone(),
        deal_start + sector_term,
    );

    // Advance time to max seal duration and prove the sector
    v = advance_by_deadline_to_epoch(v, miner_id, deal_start).0;
    miner_prove_sector(&v, worker, miner_id, sector_number);
    // Trigger cron to validate the prove commit
    cron_tick(&v);

    // Verify sector info
    let miner_state: MinerState = v.get_state(miner_id).unwrap();
    let sector_info = miner_state.get_sector(&store, sector_number).unwrap().unwrap();
    assert_eq!(sector_term, sector_info.expiration - sector_info.activation);
    assert_eq!(DealWeight::zero(), sector_info.deal_weight);
    // Verified weight is sector term * 32 GiB, using simple QAP
    let verified_weight = DealWeight::from(sector_term as u64 * deal_size);
    assert_eq!(verified_weight, sector_info.verified_deal_weight);

    // Verify deal state.
    let market_state: MarketState = v.get_state(STORAGE_MARKET_ACTOR_ADDR).unwrap();
    let deal_states = DealMetaArray::load(&market_state.states, v.store).unwrap();
    let deal_state = deal_states.get(deals[0]).unwrap().unwrap();
    let claim_id = deal_state.verified_claim;
    assert_ne!(0, claim_id);

    // Verify datacap state
    let datacap_state: DatacapState = v.get_state(DATACAP_TOKEN_ACTOR_ADDR).unwrap();
    assert_eq!(
        TokenAmount::from_whole(datacap.clone()) - TokenAmount::from_whole(deal_size), // Spent deal size
        datacap_state.token.get_balance(&v.store, verified_client.id().unwrap()).unwrap()
    );
    assert_eq!(
        TokenAmount::from_whole(datacap.clone()), // Nothing spent
        datacap_state.token.get_balance(&v.store, verified_client2.id().unwrap()).unwrap()
    );
    assert_eq!(
        TokenAmount::zero(), // Burnt when the allocation was claimed
        datacap_state
            .token
            .get_balance(&v.store, VERIFIED_REGISTRY_ACTOR_ADDR.id().unwrap())
            .unwrap()
    );
    assert_eq!(
        TokenAmount::from_whole(datacap.clone()) * 2 - TokenAmount::from_whole(deal_size),
        datacap_state.token.supply
    );

    // Verify claim state
    let verifreg_state: VerifregState = v.get_state(VERIFIED_REGISTRY_ACTOR_ADDR).unwrap();
    let mut claims = verifreg_state.load_claims(v.store).unwrap();
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
    let (deadline_info, partition_index, v) =
        advance_to_proving_deadline(v, miner_id, sector_number);

    let expected_power =
        PowerPair { raw: StoragePower::from(deal_size), qa: StoragePower::from(10 * deal_size) };
    submit_windowed_post(
        &v,
        worker,
        miner_id,
        deadline_info,
        partition_index,
        Some(expected_power.clone()),
    );

    // Verify miner power
    let power_state: PowerState = v.get_state(STORAGE_POWER_ACTOR_ADDR).unwrap();
    let power_claim = power_state.get_claim(v.store, &miner_id).unwrap().unwrap();
    assert_eq!(power_claim.raw_byte_power, expected_power.raw);
    assert_eq!(power_claim.quality_adj_power, expected_power.qa);

    // move forward one deadline so advanceWhileProving doesn't fail double submitting posts.
    let (mut v, _) = advance_by_deadline_to_index(
        v,
        miner_id,
        deadline_info.index + 1 % policy.wpost_period_deadlines,
    );

    // Advance past the deal's minimum term (the claim remains valid).
    v = advance_by_deadline_to_epoch_while_proving(
        v,
        miner_id,
        worker,
        sector_number,
        deal_start + deal_term_min + 10,
    );

    // The client extends the verified claim term out to 12 months.
    verifreg_extend_claim_terms(&v, verified_client, miner_id, claim_id, 360 * EPOCHS_IN_DAY);

    // Now the miner can extend the sector's expiration to the same.
    let (didx, pidx) = sector_deadline(&v, miner_id, sector_number);
    let extended_expiration_1 = deal_start + 360 * EPOCHS_IN_DAY;
    miner_extend_sector_expiration2(
        &v,
        worker,
        miner_id,
        didx,
        pidx,
        vec![],
        vec![SectorClaim { sector_number, maintain_claims: vec![claim_id], drop_claims: vec![] }],
        extended_expiration_1,
        PowerPair::zero(), // No change in power
    );

    // Advance toward the sector's expiration
    v = advance_by_deadline_to_epoch_while_proving(
        v,
        miner_id,
        worker,
        sector_number,
        extended_expiration_1 - 100,
    );

    // Another client extends the claim beyond the initial maximum term.
    let original_max_term = policy.maximum_verified_allocation_term;
    let new_claim_expiry_epoch = v.get_epoch() + policy.maximum_verified_allocation_term;
    let new_max_term = new_claim_expiry_epoch - claim.term_start;
    assert!(new_max_term > original_max_term);

    datacap_extend_claim(&v, verified_client2, miner_id, claim_id, deal_size, new_max_term);

    // The miner extends the sector into the second year.
    let extended_expiration_2 = extended_expiration_1 + 60 * EPOCHS_IN_DAY;
    miner_extend_sector_expiration2(
        &v,
        worker,
        miner_id,
        didx,
        pidx,
        vec![],
        vec![SectorClaim { sector_number, maintain_claims: vec![claim_id], drop_claims: vec![] }],
        extended_expiration_2,
        PowerPair::zero(), // No change in power
    );

    // Advance toward the sector's new expiration
    v = advance_by_deadline_to_epoch_while_proving(
        v,
        miner_id,
        worker,
        sector_number,
        extended_expiration_2 - 30 * EPOCHS_IN_DAY,
    );

    // The miner can drop the claim, losing the multiplied QA power.
    let expected_power_delta =
        PowerPair::new(StoragePower::zero(), StoragePower::from(9 * deal_size).neg());
    miner_extend_sector_expiration2(
        &v,
        worker,
        miner_id,
        didx,
        pidx,
        vec![],
        vec![SectorClaim { sector_number, maintain_claims: vec![], drop_claims: vec![claim_id] }],
        extended_expiration_2, // No change in expiration
        expected_power_delta,  // Power lost
    );

    // Verify sector info
    let miner_state: MinerState = v.get_state(miner_id).unwrap();
    let sector_info = miner_state.get_sector(&store, sector_number).unwrap().unwrap();
    assert_eq!(extended_expiration_2, sector_info.expiration);
    assert_eq!(DealWeight::zero(), sector_info.deal_weight);
    assert_eq!(DealWeight::zero(), sector_info.verified_deal_weight); // No longer verified

    // Verify datacap state
    let datacap_state: DatacapState = v.get_state(DATACAP_TOKEN_ACTOR_ADDR).unwrap();
    assert_eq!(
        TokenAmount::from_whole(datacap.clone()) - TokenAmount::from_whole(deal_size), // Spent deal size
        datacap_state.token.get_balance(&v.store, verified_client.id().unwrap()).unwrap()
    );
    assert_eq!(
        TokenAmount::from_whole(datacap.clone()) - TokenAmount::from_whole(deal_size), // Also spent deal size
        datacap_state.token.get_balance(&v.store, verified_client2.id().unwrap()).unwrap()
    );
    assert_eq!(
        TokenAmount::zero(), // All burnt
        datacap_state
            .token
            .get_balance(&v.store, VERIFIED_REGISTRY_ACTOR_ADDR.id().unwrap())
            .unwrap()
    );
    assert_eq!(
        TokenAmount::from_whole(datacap) * 2 - TokenAmount::from_whole(deal_size) * 2, // Spent deal size twice
        datacap_state.token.supply
    );

    // Advance sector to expiration
    v = advance_by_deadline_to_epoch_while_proving(
        v,
        miner_id,
        worker,
        sector_number,
        extended_expiration_2,
    );
    // And advance vm past the claim's max term (no more sector exists to prove)
    let v = v.with_epoch(new_claim_expiry_epoch);
    // Expired claim can now be cleaned up
    let cleanup_claims =
        RemoveExpiredClaimsParams { provider: miner_id.id().unwrap(), claim_ids: vec![claim_id] };

    let ret_raw = apply_ok(
        &v,
        worker,
        VERIFIED_REGISTRY_ACTOR_ADDR,
        TokenAmount::zero(),
        VerifregMethod::RemoveExpiredClaims as u64,
        cleanup_claims,
    );
    let ret: RemoveExpiredClaimsReturn = deserialize(&ret_raw, "balance of return value").unwrap();
    assert_eq!(vec![claim_id], ret.considered);
    assert!(ret.results.all_ok(), "results had failures {}", ret.results);

    v.expect_state_invariants(
        &[invariant_failure_patterns::REWARD_STATE_EPOCH_MISMATCH.to_owned()],
    );
}

#[test]
fn expired_allocations() {
    let store = MemoryBlockstore::new();
    let mut v = VM::new_with_singletons(&store);
    let addrs = create_accounts(&v, 3, TokenAmount::from_whole(10_000));
    let seal_proof = RegisteredSealProof::StackedDRG32GiBV1P1;
    let (owner, worker, verifier, verified_client) = (addrs[0], addrs[0], addrs[1], addrs[2]);

    // Create miner
    let (miner_id, _) = create_miner(
        &mut v,
        owner,
        worker,
        seal_proof.registered_window_post_proof().unwrap(),
        TokenAmount::from_whole(1_000),
    );
    let v = v.with_epoch(200);

    // Register verifier and verified clients
    let datacap = StoragePower::from(32_u128 << 40);
    verifreg_add_verifier(&v, verifier, &datacap * 2);
    verifreg_add_client(&v, verifier, verified_client, datacap.clone());

    // Add market collateral for client and miner
    market_add_balance(&v, verified_client, verified_client, TokenAmount::from_whole(3));
    market_add_balance(&v, worker, miner_id, TokenAmount::from_whole(64));

    // Publish 2 verified deals
    let deal1_start =
        v.get_epoch() + max_prove_commit_duration(&Policy::default(), seal_proof).unwrap();
    let deal_term_min = 180 * EPOCHS_IN_DAY;

    let deal_size = 32u64 << 30;
    let deal1 = market_publish_deal(
        &v,
        worker,
        verified_client,
        miner_id,
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
        datacap_get_balance(&v, verified_client)
    );

    // Advance to after the first deal's start
    let v = v.with_epoch(deal1_start + DEAL_UPDATES_INTERVAL);
    cron_tick(&v);

    // Deal has expired and cleaned up.
    let market_state: MarketState = v.get_state(STORAGE_MARKET_ACTOR_ADDR).unwrap();
    let proposals: DealArray<MemoryBlockstore> =
        DealArray::load(&market_state.proposals, v.store).unwrap();
    assert!(proposals.get(deal1).unwrap().is_none());
    let pending_deal_allocs: Map<MemoryBlockstore, AllocationID> = make_map_with_root_and_bitwidth(
        &market_state.pending_deal_allocation_ids,
        v.store,
        HAMT_BIT_WIDTH,
    )
    .unwrap();
    assert!(pending_deal_allocs.get(&deal_id_key(deal1)).unwrap().is_none());

    // Allocation still exists until explicit cleanup
    let alloc_id = 1;
    let verifreg_state: VerifregState = v.get_state(VERIFIED_REGISTRY_ACTOR_ADDR).unwrap();
    let mut allocs = verifreg_state.load_allocs(v.store).unwrap();
    assert!(allocs.get(verified_client.id().unwrap(), alloc_id).unwrap().is_some());

    verifreg_remove_expired_allocations(&v, worker, verified_client, vec![], deal_size);

    // Allocation is gone
    let verifreg_state: VerifregState = v.get_state(VERIFIED_REGISTRY_ACTOR_ADDR).unwrap();
    let mut allocs = verifreg_state.load_allocs(v.store).unwrap();
    assert!(allocs.get(verified_client.id().unwrap(), alloc_id).unwrap().is_none());

    // Client has original datacap balance
    assert_eq!(TokenAmount::from_whole(datacap), datacap_get_balance(&v, verified_client));

    v.expect_state_invariants(
        &[invariant_failure_patterns::REWARD_STATE_EPOCH_MISMATCH.to_owned()],
    );
}
