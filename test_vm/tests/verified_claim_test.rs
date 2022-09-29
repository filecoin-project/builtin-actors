use fvm_ipld_blockstore::MemoryBlockstore;
use fvm_shared::bigint::Zero;
use fvm_shared::econ::TokenAmount;
use fvm_shared::piece::PaddedPieceSize;
use fvm_shared::sector::{RegisteredSealProof, SectorNumber, StoragePower};

use fil_actor_market::DealMetaArray;
use fil_actor_market::State as MarketState;
use fil_actor_miner::{max_prove_commit_duration, PowerPair, SectorClaim, State as MinerState};
use fil_actor_verifreg::{Claim, State as VerifregState};
use fil_actors_runtime::runtime::Policy;
use fil_actors_runtime::test_utils::make_piece_cid;
use fil_actors_runtime::{
    DealWeight, EPOCHS_IN_DAY, STORAGE_MARKET_ACTOR_ADDR, VERIFIED_REGISTRY_ACTOR_ADDR,
};
use test_vm::util::{
    advance_by_deadline_to_epoch, advance_by_deadline_to_epoch_while_proving,
    advance_by_deadline_to_index, advance_to_proving_deadline, create_accounts, create_miner,
    cron_tick, market_add_balance, market_publish_deal, miner_extend_sector_expiration2,
    miner_precommit_sector, miner_prove_sector, sector_deadline, submit_windowed_post,
    verifreg_add_client, verifreg_add_verifier, verifreg_extend_claim_terms,
};
use test_vm::VM;

// Tests a scenario involving a verified deal from the built-in market, with associated
// allocation and claim.
// This test shares some set-up copied from extend_sectors_test.
#[test]
fn verified_claim_scenario() {
    let store = MemoryBlockstore::new();
    let mut v = VM::new_with_singletons(&store);
    let addrs = create_accounts(&v, 3, TokenAmount::from_whole(10_000));
    let seal_proof = RegisteredSealProof::StackedDRG32GiBV1P1;
    let (owner, worker, verifier, verified_client) = (addrs[0], addrs[0], addrs[1], addrs[2]);
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

    // Register verifier and verified client
    let datacap = StoragePower::from(32_u128 << 40);
    verifreg_add_verifier(&v, verifier, datacap.clone());
    verifreg_add_client(&v, verifier, verified_client, datacap);

    // Add market collateral for clients and miner
    market_add_balance(&v, verified_client, verified_client, TokenAmount::from_whole(3));
    market_add_balance(&v, worker, miner_id, TokenAmount::from_whole(64));

    // Publish a verified deal for total sector capacity with min term of 6 months
    let deal_start =
        v.get_epoch() + max_prove_commit_duration(&Policy::default(), seal_proof).unwrap();
    let deal_term_min = 180 * EPOCHS_IN_DAY;

    let deal_size = PaddedPieceSize(32u64 << 30);
    let deals = market_publish_deal(
        &v,
        worker,
        verified_client,
        miner_id,
        "deal1".to_string(),
        deal_size,
        true,
        deal_start,
        deal_term_min,
    )
    .ids;

    // Precommit and prove the sector
    let sector_term = 240 * EPOCHS_IN_DAY; // Longer than deal min term, but less than max.
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
    assert_eq!(StoragePower::zero(), sector_info.deal_weight);
    // Verified weight is sector term * 32 GiB, using simple QAP
    let verified_weight = DealWeight::from(sector_term * (32i64 << 30));
    assert_eq!(verified_weight, sector_info.verified_deal_weight);

    // Verify deal state.
    let market_state: MarketState = v.get_state(STORAGE_MARKET_ACTOR_ADDR).unwrap();
    let deal_states = DealMetaArray::load(&market_state.states, v.store).unwrap();
    let deal_state = deal_states.get(deals[0]).unwrap().unwrap();
    let claim_id = deal_state.verified_claim;
    assert_ne!(0, claim_id);

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
            size: deal_size,
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

    let expected_power_delta = PowerPair {
        raw: StoragePower::from(32u64 << 30),
        qa: StoragePower::from(10 * (32u64 << 30)),
    };
    submit_windowed_post(
        &v,
        worker,
        miner_id,
        deadline_info,
        partition_index,
        Some(expected_power_delta),
    );

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
    miner_extend_sector_expiration2(
        &v,
        worker,
        miner_id,
        didx,
        pidx,
        vec![],
        vec![SectorClaim { sector_number, maintain_claims: vec![claim_id], drop_claims: vec![] }],
        deal_start + 360 * EPOCHS_IN_DAY,
        PowerPair::zero(), // No change in power
    );

    // TODO continuing this test
    //  - extend the claim further by spending more datacap
    //  - extend sector again
    //  - drop the claim while extending sector
}
