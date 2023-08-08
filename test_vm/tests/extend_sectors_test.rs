use fil_actor_miner::{max_prove_commit_duration, PowerPair, Sectors, State as MinerState};
use fil_actors_integration_tests::tests::{
    commit_sector_with_max_duration_deal_test, extend,
    extend_sector_up_to_max_relative_extension_test, extend_updated_sector_with_claims_test,
};
use fil_actors_integration_tests::util::{
    advance_by_deadline_to_epoch, advance_by_deadline_to_epoch_while_proving,
    advance_by_deadline_to_index, advance_to_proving_deadline, create_accounts, create_miner,
    cron_tick, expect_invariants, invariant_failure_patterns, market_add_balance,
    market_publish_deal, miner_precommit_one_sector_v2, miner_prove_sector,
    precommit_meta_data_from_deals, submit_windowed_post, verifreg_add_client,
    verifreg_add_verifier,
};
use fil_actors_runtime::runtime::Policy;
use fil_actors_runtime::{DealWeight, EPOCHS_IN_DAY};
use fvm_ipld_blockstore::{Blockstore, MemoryBlockstore};
use fvm_shared::bigint::Zero;
use fvm_shared::econ::TokenAmount;
use fvm_shared::piece::PaddedPieceSize;
use fvm_shared::sector::{RegisteredSealProof, SectorNumber, StoragePower};
use test_vm::TestVM;
use vm_api::util::{get_state, DynBlockstore};
use vm_api::VM;

#[test]
fn extend_legacy_sector_with_deals() {
    let store = MemoryBlockstore::new();
    let v = TestVM::<MemoryBlockstore>::new_with_singletons(&store);
    extend_legacy_sector_with_deals_inner(&v, false, &v);
}

#[test]
fn extend2_legacy_sector_with_deals() {
    let store = MemoryBlockstore::new();
    let v = TestVM::<MemoryBlockstore>::new_with_singletons(&store);
    extend_legacy_sector_with_deals_inner(&v, true, &v);
}

// TODO: remove usage of _v_concrete which is currently required by mutate_state
// https://github.com/filecoin-project/builtin-actors/issues/1297
fn extend_legacy_sector_with_deals_inner<BS: Blockstore>(
    v: &dyn VM,
    do_extend2: bool,
    _v_concrete: &TestVM<BS>,
) {
    let addrs = create_accounts(v, 3, &TokenAmount::from_whole(10_000));
    let seal_proof = RegisteredSealProof::StackedDRG32GiBV1P1;
    let (owner, worker, verifier, verified_client) = (addrs[0], addrs[0], addrs[1], addrs[2]);
    let sector_number: SectorNumber = 100;
    let policy = Policy::default();

    // create miner
    let miner_id = create_miner(
        v,
        &owner,
        &worker,
        seal_proof.registered_window_post_proof().unwrap(),
        &TokenAmount::from_whole(1_000),
    )
    .0;
    v.set_epoch(200);

    //
    // publish verified deals
    //

    // register verifier then verified client
    let datacap = StoragePower::from(32_u128 << 40);
    verifreg_add_verifier(v, &verifier, datacap.clone());
    verifreg_add_client(v, &verifier, &verified_client, datacap);

    // add market collateral for clients and miner
    market_add_balance(v, &verified_client, &verified_client, &TokenAmount::from_whole(3));
    market_add_balance(v, &worker, &miner_id, &TokenAmount::from_whole(64));

    // create 1 verified deal for total sector capacity for 6 months
    let deal_start = v.epoch() + max_prove_commit_duration(&Policy::default(), seal_proof).unwrap();
    let deals = market_publish_deal(
        v,
        &worker,
        &verified_client,
        &miner_id,
        "deal1".to_string(),
        PaddedPieceSize(32u64 << 30),
        true,
        deal_start,
        180 * EPOCHS_IN_DAY,
    )
    .ids;

    //
    // Precommit, prove and PoSt empty sector (more fully tested in TestCommitPoStFlow)
    //

    miner_precommit_one_sector_v2(
        v,
        &worker,
        &miner_id,
        seal_proof,
        sector_number,
        precommit_meta_data_from_deals(v, deals, seal_proof),
        true,
        deal_start + 180 * EPOCHS_IN_DAY,
    );

    // advance time to max seal duration and prove the sector
    advance_by_deadline_to_epoch(v, &miner_id, deal_start);
    miner_prove_sector(v, &worker, &miner_id, sector_number);
    // trigger cron to validate the prove commit
    cron_tick(v);

    // inspect sector info

    let miner_state: MinerState = get_state(v, &miner_id).unwrap();
    let mut sector_info = miner_state
        .get_sector(&DynBlockstore::wrap(v.blockstore()), sector_number)
        .unwrap()
        .unwrap();
    assert_eq!(180 * EPOCHS_IN_DAY, sector_info.expiration - sector_info.activation);
    assert_eq!(StoragePower::zero(), sector_info.deal_weight); // 0 space time
    assert_eq!(
        DealWeight::from(180 * EPOCHS_IN_DAY * (32i64 << 30)),
        sector_info.verified_deal_weight
    ); // (180 days *2880 epochs per day) * 32 GiB

    // Note: we don't need to explicitly set verified weight using the legacy method
    // because legacy and simple qa power deal weight calculations line up for fully packed sectors
    // We do need to set simple_qa_power to false
    sector_info.simple_qa_power = false;

    // Manually craft state to match legacy sectors
    _v_concrete.mutate_state(&miner_id, |st: &mut MinerState| {
        let store = &DynBlockstore::wrap(v.blockstore());
        let mut sectors = Sectors::load(store, &st.sectors).unwrap();
        sectors.store(vec![sector_info.clone()]).unwrap();
        st.sectors = sectors.amt.flush().unwrap();
    });

    let initial_verified_deal_weight = sector_info.verified_deal_weight;
    let initial_deal_weight = sector_info.deal_weight;

    // advance to proving period and submit post
    let (deadline_info, partition_index) = advance_to_proving_deadline(v, &miner_id, sector_number);
    let expected_power_delta = PowerPair {
        raw: StoragePower::from(32u64 << 30),
        qa: StoragePower::from(10 * (32u64 << 30)),
    };
    submit_windowed_post(
        v,
        &worker,
        &miner_id,
        deadline_info,
        partition_index,
        Some(expected_power_delta),
    );

    // move forward one deadline so advanceWhileProving doesn't fail double submitting posts
    advance_by_deadline_to_index(
        v,
        &miner_id,
        deadline_info.index + 1 % policy.wpost_period_deadlines,
    );

    // Advance halfway through life and extend another 6 months. We need to spread the remaining 90
    // days of 10x power over 90 + 180 days
    // subtract half the remaining deal weight:
    //   - verified deal weight /= 2
    //
    // normalize 90 days of 10x power plus 180 days of 1x power over 90+180 days:
    //   - multiplier = ((10 * 90) + (1 * 180)) / (90 + 180)
    //   - multiplier = 4
    //
    // delta from the previous 10x power multiplier:
    // - power delta = (10-4)*32GiB = 6*32GiB
    advance_by_deadline_to_epoch_while_proving(
        v,
        &miner_id,
        &worker,
        sector_number,
        deal_start + 90 * EPOCHS_IN_DAY,
    );

    let new_expiration = deal_start + 2 * 180 * EPOCHS_IN_DAY;
    let expected_power_delta =
        PowerPair { raw: StoragePower::zero(), qa: StoragePower::from(-6 * (32i64 << 30)) };
    extend(
        v,
        worker,
        miner_id,
        deadline_info.index,
        partition_index,
        sector_number,
        new_expiration,
        expected_power_delta,
        do_extend2,
    );

    let miner_state: MinerState = get_state(v, &miner_id).unwrap();
    sector_info = miner_state
        .get_sector(&DynBlockstore::wrap(v.blockstore()), sector_number)
        .unwrap()
        .unwrap();
    assert_eq!(180 * 2 * EPOCHS_IN_DAY, sector_info.expiration - sector_info.activation);
    assert_eq!(initial_deal_weight, sector_info.deal_weight); // 0 space time, unchanged
    assert_eq!(&initial_verified_deal_weight / 2, sector_info.verified_deal_weight);

    // advance to 6 months (original expiration) and extend another 6 months
    //
    // We're 1/3rd of the way through the last extension, so keep 2/3 of the power.
    //   - verified deal weight *= 2/3
    //
    // normalize 180 days of 4x power plus 180 days of 1x power over 180+180 days:
    //   - multiplier = ((4 * 180) + (1 * 180)) / (90 + 180)
    //   - multiplier = 2.5
    //
    // delta from the previous 4x power multiplier:
    // - power delta = (4-2.5)*32GiB = 1.5*32GiB

    advance_by_deadline_to_epoch_while_proving(
        v,
        &miner_id,
        &worker,
        sector_number,
        deal_start + 180 * EPOCHS_IN_DAY,
    );

    let new_expiration = deal_start + 3 * 180 * EPOCHS_IN_DAY;
    let expected_power_delta =
        PowerPair { raw: StoragePower::zero(), qa: StoragePower::from(-15 * (32i64 << 30) / 10) };
    extend(
        v,
        worker,
        miner_id,
        deadline_info.index,
        partition_index,
        sector_number,
        new_expiration,
        expected_power_delta,
        do_extend2,
    );

    let miner_state: MinerState = get_state(v, &miner_id).unwrap();
    let sector_info = miner_state
        .get_sector(&DynBlockstore::wrap(v.blockstore()), sector_number)
        .unwrap()
        .unwrap();
    assert_eq!(180 * 3 * EPOCHS_IN_DAY, sector_info.expiration - sector_info.activation);
    // 0 space time, unchanged
    assert_eq!(initial_deal_weight, sector_info.deal_weight);
    // 1/2 * 2/3 -> 1/3
    assert_eq!(initial_verified_deal_weight / 3, sector_info.verified_deal_weight);

    expect_invariants(
        v,
        &Policy::default(),
        &[invariant_failure_patterns::REWARD_STATE_EPOCH_MISMATCH.to_owned()],
    );
}

#[test]
fn extend_updated_sector_with_claim() {
    let store = MemoryBlockstore::new();
    let v = TestVM::<MemoryBlockstore>::new_with_singletons(&store);
    extend_updated_sector_with_claims_test(&v);
}

#[test]
fn extend_sector_up_to_max_relative_extension() {
    let store = MemoryBlockstore::new();
    let v = TestVM::<MemoryBlockstore>::new_with_singletons(&store);
    extend_sector_up_to_max_relative_extension_test(&v);
}

#[test]
fn commit_sector_with_max_duration_deal() {
    let store = MemoryBlockstore::new();
    let v = TestVM::<MemoryBlockstore>::new_with_singletons(&store);
    commit_sector_with_max_duration_deal_test(&v);
}
