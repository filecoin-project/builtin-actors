use cid::Cid;
use export_macro::vm_test;
use fil_actor_verifreg::Method as VerifregMethod;
use fil_actors_runtime::runtime::Policy;
use fil_actors_runtime::test_utils::{make_piece_cid, make_sealed_cid};
use fil_actors_runtime::{DealWeight, EPOCHS_IN_DAY, VERIFIED_REGISTRY_ACTOR_ADDR};
use fvm_ipld_bitfield::BitField;
use fvm_shared::address::Address;
use fvm_shared::bigint::Zero;
use fvm_shared::clock::ChainEpoch;
use fvm_shared::econ::TokenAmount;
use fvm_shared::piece::{PaddedPieceSize, PieceInfo};
use fvm_shared::sector::{RegisteredSealProof, SectorNumber, StoragePower};

use fil_actor_miner::{
    max_prove_commit_duration, power_for_sector, ExpirationExtension, ExpirationExtension2,
    ExtendSectorExpiration2Params, ExtendSectorExpirationParams, Method as MinerMethod, PowerPair,
    ProveReplicaUpdatesParams, ReplicaUpdate, SectorClaim, SectorOnChainInfoFlags, Sectors,
    State as MinerState,
};
use fil_actors_runtime::runtime::policy_constants::MARKET_DEFAULT_ALLOCATION_TERM_BUFFER;
use vm_api::trace::ExpectInvocation;
use vm_api::util::{apply_ok, get_state, mutate_state, DynBlockstore};
use vm_api::VM;

use crate::expects::Expect;
use crate::util::{
    advance_by_deadline_to_epoch, advance_by_deadline_to_epoch_while_proving,
    advance_by_deadline_to_index, advance_to_proving_deadline, bf_all, create_accounts,
    create_miner, cron_tick, expect_invariants, invariant_failure_patterns,
    make_piece_manifests_from_deal_ids, market_add_balance, market_pending_deal_allocations,
    market_publish_deal, miner_precommit_one_sector_v2, miner_prove_sector,
    override_compute_unsealed_sector_cid, precommit_meta_data_from_deals, sector_deadline,
    submit_windowed_post, verifreg_add_client, verifreg_add_verifier, PrecommitMetadata,
};

#[allow(clippy::too_many_arguments)]
pub fn extend(
    v: &dyn VM,
    worker: Address,
    maddr: Address,
    deadline_index: u64,
    partition_index: u64,
    sector_number: SectorNumber,
    new_expiration: ChainEpoch,
    power_delta: PowerPair,
    v2: bool,
) {
    let extension_method = match v2 {
        false => MinerMethod::ExtendSectorExpiration as u64,
        true => MinerMethod::ExtendSectorExpiration2 as u64,
    };

    let miner_id = v.resolve_id_address(&maddr).unwrap().id().unwrap();
    let worker_id = v.resolve_id_address(&worker).unwrap().id().unwrap();

    match v2 {
        false => {
            let extension_params = ExtendSectorExpirationParams {
                extensions: vec![ExpirationExtension {
                    deadline: deadline_index,
                    partition: partition_index,
                    sectors: BitField::try_from_bits([sector_number].iter().copied()).unwrap(),
                    new_expiration,
                }],
            };
            apply_ok(
                v,
                &worker,
                &maddr,
                &TokenAmount::zero(),
                extension_method,
                Some(extension_params),
            );
        }
        true => {
            let extension_params = ExtendSectorExpiration2Params {
                extensions: vec![ExpirationExtension2 {
                    deadline: deadline_index,
                    partition: partition_index,
                    sectors: BitField::try_from_bits([sector_number].iter().copied()).unwrap(),
                    new_expiration,
                    sectors_with_claims: vec![],
                }],
            };
            apply_ok(
                v,
                &worker,
                &maddr,
                &TokenAmount::zero(),
                extension_method,
                Some(extension_params),
            );
        }
    };

    let mut expect_invoke =
        vec![Expect::reward_this_epoch(miner_id), Expect::power_current_total(miner_id)];

    if !power_delta.is_zero() {
        expect_invoke.push(Expect::power_update_claim(miner_id, power_delta));
    }

    ExpectInvocation {
        from: worker_id,
        to: maddr,
        method: extension_method,
        subinvocs: Some(expect_invoke),
        ..Default::default()
    }
    .matches(v.take_invocations().last().unwrap());
}

pub fn extend_legacy_sector_with_deals_test(v: &dyn VM, do_extend2: bool) {
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
        precommit_meta_data_from_deals(v, &deals, seal_proof, false),
        true,
        deal_start + 180 * EPOCHS_IN_DAY,
    );

    // advance time to max seal duration and prove the sector
    advance_by_deadline_to_epoch(v, &miner_id, deal_start);
    miner_prove_sector(
        v,
        &worker,
        &miner_id,
        sector_number,
        make_piece_manifests_from_deal_ids(v, deals),
    );
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
    sector_info.flags.set(SectorOnChainInfoFlags::SIMPLE_QA_POWER, false);

    // Manually craft state to match legacy sectors
    mutate_state(v, &miner_id, |st: &mut MinerState| {
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
        None,
    );
}

#[vm_test]
pub fn extend_legacy_sector_with_deals_extend2(v: &dyn VM) {
    extend_legacy_sector_with_deals_test(v, true);
}

#[vm_test]
pub fn extend_legacy_sector_with_deals(v: &dyn VM) {
    extend_legacy_sector_with_deals_test(v, false);
}

#[vm_test]
pub fn commit_sector_with_max_duration_deal_test(v: &dyn VM) {
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

    let deal_lifetime = policy.max_sector_expiration_extension
        - max_prove_commit_duration(&policy, seal_proof).unwrap_or_default();
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
        deal_lifetime,
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
        precommit_meta_data_from_deals(v, &deals, seal_proof, false),
        true,
        deal_start + deal_lifetime,
    );

    // advance time to max seal duration and prove the sector
    advance_by_deadline_to_epoch(v, &miner_id, deal_start);
    miner_prove_sector(
        v,
        &worker,
        &miner_id,
        sector_number,
        make_piece_manifests_from_deal_ids(v, deals),
    );
    // trigger cron to validate the prove commit
    cron_tick(v);

    // advance to proving period and submit post
    let (deadline_info, partition_index) = advance_to_proving_deadline(v, &miner_id, sector_number);
    let expected_power_delta = PowerPair {
        raw: StoragePower::from(32u64 << 30),
        qa: 10 * StoragePower::from(32u64 << 30),
    };
    submit_windowed_post(
        v,
        &worker,
        &miner_id,
        deadline_info,
        partition_index,
        Some(expected_power_delta),
    );
    // inspect sector info
    let miner_state: MinerState = get_state(v, &miner_id).unwrap();
    let sector_info = miner_state
        .get_sector(&DynBlockstore::wrap(v.blockstore()), sector_number)
        .unwrap()
        .unwrap();
    assert_eq!(deal_lifetime, sector_info.expiration - sector_info.activation);
}

#[vm_test]
pub fn extend_sector_up_to_max_relative_extension_test(v: &dyn VM) {
    let addrs = create_accounts(v, 3, &TokenAmount::from_whole(10_000));
    let seal_proof = RegisteredSealProof::StackedDRG32GiBV1P1;
    let (owner, worker) = (addrs[0], addrs[0]);
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
    // Precommit, prove and PoSt empty sector (more fully tested in TestCommitPoStFlow)
    //
    let sector_start =
        v.epoch() + max_prove_commit_duration(&Policy::default(), seal_proof).unwrap();

    miner_precommit_one_sector_v2(
        v,
        &worker,
        &miner_id,
        seal_proof,
        sector_number,
        PrecommitMetadata::default(),
        true,
        sector_start + 180 * EPOCHS_IN_DAY,
    );

    // advance time to max seal duration and prove the sector
    advance_by_deadline_to_epoch(v, &miner_id, sector_start);
    miner_prove_sector(v, &worker, &miner_id, sector_number, vec![]);
    // trigger cron to validate the prove commit
    cron_tick(v);

    // inspect sector info
    let mut miner_state: MinerState = get_state(v, &miner_id).unwrap();
    let mut sector_info = miner_state
        .get_sector(&DynBlockstore::wrap(v.blockstore()), sector_number)
        .unwrap()
        .unwrap();
    assert_eq!(180 * EPOCHS_IN_DAY, sector_info.expiration - sector_info.activation);

    // advance to proving period and submit post
    let (deadline_info, partition_index) = advance_to_proving_deadline(v, &miner_id, sector_number);
    let expected_power_delta =
        PowerPair { raw: StoragePower::from(32u64 << 30), qa: StoragePower::from(32u64 << 30) };
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

    // Extend the sector by the max relative extension.
    let new_expiration = v.epoch() + policy.max_sector_expiration_extension;
    let expected_power_delta = PowerPair::zero();
    extend(
        v,
        worker,
        miner_id,
        deadline_info.index,
        partition_index,
        sector_number,
        new_expiration,
        expected_power_delta,
        false,
    );

    miner_state = get_state(v, &miner_id).unwrap();
    sector_info = miner_state
        .get_sector(&DynBlockstore::wrap(v.blockstore()), sector_number)
        .unwrap()
        .unwrap();
    assert_eq!(policy.max_sector_expiration_extension, sector_info.expiration - v.epoch());
}

#[vm_test]
pub fn extend_updated_sector_with_claims_test(v: &dyn VM) {
    override_compute_unsealed_sector_cid(v);

    let addrs = create_accounts(v, 3, &TokenAmount::from_whole(10_000));
    let seal_proof = RegisteredSealProof::StackedDRG32GiBV1P1;
    let (owner, worker, verifier, verified_client) = (addrs[0], addrs[0], addrs[1], addrs[2]);
    let worker_id = worker.id().unwrap();
    let sector_number: SectorNumber = 100;
    let policy = Policy::default();
    let deal_label = "deal1".to_string();
    let piece_cid = make_piece_cid(deal_label.as_bytes());
    let piece_size = PaddedPieceSize(32u64 << 30);

    // create miner
    let miner_addr = create_miner(
        v,
        &owner,
        &worker,
        seal_proof.registered_window_post_proof().unwrap(),
        &TokenAmount::from_whole(1_000),
    )
    .0;
    let miner_id = miner_addr.id().unwrap();
    v.set_epoch(200);

    //
    // Precommit, prove and PoSt empty sector (more fully tested in TestCommitPoStFlow)
    //

    let expiration = v.epoch() + 360 * EPOCHS_IN_DAY;

    miner_precommit_one_sector_v2(
        v,
        &worker,
        &miner_addr,
        seal_proof,
        sector_number,
        PrecommitMetadata::default(),
        true,
        expiration,
    );

    // advance time by a day and prove the sector
    let prove_epoch = v.epoch() + EPOCHS_IN_DAY;
    advance_by_deadline_to_epoch(v, &miner_addr, prove_epoch);
    miner_prove_sector(v, &worker, &miner_addr, sector_number, vec![]);
    // trigger cron to validate the prove commit
    cron_tick(v);

    // advance to proving period and submit post

    let (deadline_info, partition_index) =
        advance_to_proving_deadline(v, &miner_addr, sector_number);
    let expected_power_delta =
        PowerPair { raw: StoragePower::from(32u64 << 30), qa: StoragePower::from(32u64 << 30) };
    submit_windowed_post(
        v,
        &worker,
        &miner_addr,
        deadline_info,
        partition_index,
        Some(expected_power_delta),
    );

    // move forward one deadline so sector is mutable
    advance_by_deadline_to_index(
        v,
        &miner_addr,
        deadline_info.index + 1 % policy.wpost_period_deadlines,
    );

    // Inspect basic sector info

    let miner_state: MinerState = get_state(v, &miner_addr).unwrap();
    let initial_sector_info = miner_state
        .get_sector(&DynBlockstore::wrap(v.blockstore()), sector_number)
        .unwrap()
        .unwrap();
    assert_eq!(expiration, initial_sector_info.expiration);
    assert!(initial_sector_info.deal_weight.is_zero());
    // 0 space time
    assert!(initial_sector_info.verified_deal_weight.is_zero());
    // 0 space time
    // publish verified deals

    // register verifier then verified client
    let datacap = StoragePower::from(32_u128 << 40);
    verifreg_add_verifier(v, &verifier, datacap.clone());
    verifreg_add_client(v, &verifier, &verified_client, datacap);

    // add market collateral for clients and miner
    market_add_balance(v, &verified_client, &verified_client, &TokenAmount::from_whole(3));
    market_add_balance(v, &worker, &miner_addr, &TokenAmount::from_whole(64));

    // create 1 verified deal for total sector capacity
    let deal_start = v.epoch() + EPOCHS_IN_DAY;
    let deal_lifetime = 340 * EPOCHS_IN_DAY;
    let deal_ids = market_publish_deal(
        v,
        &worker,
        &verified_client,
        &miner_addr,
        deal_label,
        piece_size,
        true,
        deal_start,
        deal_lifetime,
    )
    .ids;

    let claim_id = market_pending_deal_allocations(v, &deal_ids)[0];

    // replica update
    let new_sealed_cid = make_sealed_cid(b"replica1");

    let (d_idx, p_idx) = sector_deadline(v, &miner_addr, sector_number);
    let replica_update = ReplicaUpdate {
        sector_number,
        deadline: d_idx,
        partition: p_idx,
        new_sealed_cid,
        deals: deal_ids.clone(),
        update_proof_type: fvm_shared::sector::RegisteredUpdateProof::StackedDRG32GiBV1,
        replica_proof: vec![].into(),
    };
    let updated_sectors: BitField = apply_ok(
        v,
        &worker,
        &miner_addr,
        &TokenAmount::zero(),
        MinerMethod::ProveReplicaUpdates as u64,
        Some(ProveReplicaUpdatesParams { updates: vec![replica_update] }),
    )
    .deserialize()
    .unwrap();
    assert_eq!(vec![sector_number], bf_all(updated_sectors));

    let old_power = power_for_sector(seal_proof.sector_size().unwrap(), &initial_sector_info);

    let pieces: Vec<(Cid, u64)> = vec![(piece_cid, piece_size.0)];
    let pis: Vec<PieceInfo> = vec![PieceInfo { cid: piece_cid, size: piece_size }];
    let unsealed_cid = v.primitives().compute_unsealed_sector_cid(seal_proof, &pis).unwrap();

    let start_epoch = deal_start;
    let end_epoch = deal_start + deal_lifetime;
    let claim_term = end_epoch - start_epoch;

    // check for the expected subcalls
    ExpectInvocation {
        from: worker_id,
        to: miner_addr,
        method: MinerMethod::ProveReplicaUpdates as u64,
        subinvocs: Some(vec![
            Expect::market_activate_deals(
                miner_id,
                deal_ids,
                verified_client.id().unwrap(),
                sector_number,
                initial_sector_info.expiration,
                initial_sector_info.seal_proof,
                true,
            ),
            ExpectInvocation {
                from: miner_id,
                to: VERIFIED_REGISTRY_ACTOR_ADDR,
                method: VerifregMethod::ClaimAllocations as u64,
                events: Some(vec![Expect::build_verifreg_claim_event(
                    "claim",
                    claim_id,
                    verified_client.id().unwrap(),
                    miner_id,
                    &piece_cid,
                    piece_size.0,
                    claim_term,
                    claim_term + MARKET_DEFAULT_ALLOCATION_TERM_BUFFER,
                    v.epoch(),
                    sector_number,
                )]),
                ..Default::default()
            },
            Expect::reward_this_epoch(miner_id),
            Expect::power_current_total(miner_id),
            Expect::power_update_pledge(miner_id, None),
            Expect::power_update_claim(
                miner_id,
                PowerPair { raw: StoragePower::zero(), qa: 9 * old_power.qa },
            ),
        ]),
        events: Some(vec![Expect::build_sector_activation_event(
            "sector-updated",
            miner_id,
            sector_number,
            Some(unsealed_cid),
            &pieces,
        )]),
        ..Default::default()
    }
    .matches(v.take_invocations().last().unwrap());

    // inspect sector info

    let miner_state: MinerState = get_state(v, &miner_addr).unwrap();
    let sector_info_after_update = miner_state
        .get_sector(&DynBlockstore::wrap(v.blockstore()), sector_number)
        .unwrap()
        .unwrap();
    // 0 space time
    assert_eq!(StoragePower::zero(), sector_info_after_update.deal_weight);

    // 32 GiB * the remaining life of the sector
    assert_eq!(
        DealWeight::from((sector_info_after_update.expiration - v.epoch()) * (32i64 << 30)),
        sector_info_after_update.verified_deal_weight
    );

    // power base epoch is updated correctly
    assert_eq!(v.epoch(), sector_info_after_update.power_base_epoch);

    // activation not changed
    assert_eq!(initial_sector_info.activation, sector_info_after_update.activation);

    // replaced day reward updated
    assert_eq!(
        initial_sector_info.expected_day_reward,
        sector_info_after_update.replaced_day_reward
    );

    // extend the updated sector

    // For clarity in checking power_base_epoch, we increment epoch by 1
    let curr_epoch = v.epoch();
    v.set_epoch(curr_epoch + 1);

    let extension_params = ExtendSectorExpiration2Params {
        extensions: vec![ExpirationExtension2 {
            deadline: d_idx,
            partition: partition_index,
            sectors: BitField::new(),
            new_expiration: sector_info_after_update.expiration + 60 * EPOCHS_IN_DAY,
            sectors_with_claims: vec![SectorClaim {
                sector_number,
                maintain_claims: vec![claim_id],
                drop_claims: vec![],
            }],
        }],
    };
    apply_ok(
        v,
        &worker,
        &miner_addr,
        &TokenAmount::zero(),
        MinerMethod::ExtendSectorExpiration2 as u64,
        Some(extension_params),
    );

    let miner_state: MinerState = get_state(v, &miner_addr).unwrap();
    let sector_info_after_extension = miner_state
        .get_sector(&DynBlockstore::wrap(v.blockstore()), sector_number)
        .unwrap()
        .unwrap();
    // 0 space time
    assert_eq!(StoragePower::zero(), sector_info_after_extension.deal_weight);

    // 32 GiB * the remaining life of the sector
    assert_eq!(
        DealWeight::from((sector_info_after_extension.expiration - v.epoch()) * (32i64 << 30)),
        sector_info_after_extension.verified_deal_weight
    );

    assert_eq!(sector_info_after_extension.power_base_epoch, v.epoch());
    assert_eq!(sector_info_after_update.activation, sector_info_after_extension.activation);
    assert_eq!(
        sector_info_after_extension.replaced_day_reward,
        sector_info_after_update.expected_day_reward
    );
}
