use fil_actor_market::{DealMetaArray, Method as MarketMethod, State as MarketState};
use fil_actor_miner::{
    max_prove_commit_duration, power_for_sector, ExpirationExtension, ExpirationExtension2,
    ExtendSectorExpiration2Params, ExtendSectorExpirationParams, Method as MinerMethod, PowerPair,
    ProveReplicaUpdatesParams2, ReplicaUpdate2, SectorClaim, Sectors, State as MinerState,
};
use fil_actor_power::{Method as PowerMethod, UpdateClaimedPowerParams};
use fil_actor_reward::Method as RewardMethod;
use fil_actor_verifreg::Method as VerifregMethod;
use fil_actors_runtime::runtime::Policy;
use fil_actors_runtime::test_utils::{make_piece_cid, make_sealed_cid};
use fil_actors_runtime::{
    DealWeight, EPOCHS_IN_DAY, REWARD_ACTOR_ADDR, STORAGE_MARKET_ACTOR_ADDR,
    STORAGE_POWER_ACTOR_ADDR, VERIFIED_REGISTRY_ACTOR_ADDR,
};
use fvm_ipld_bitfield::BitField;
use fvm_ipld_blockstore::{Blockstore, MemoryBlockstore};
use fvm_ipld_encoding::ipld_block::IpldBlock;
use fvm_shared::address::Address;
use fvm_shared::bigint::Zero;
use fvm_shared::clock::ChainEpoch;
use fvm_shared::econ::TokenAmount;
use fvm_shared::piece::PaddedPieceSize;
use fvm_shared::sector::{RegisteredSealProof, SectorNumber, StoragePower};
use test_vm::util::{
    advance_by_deadline_to_epoch, advance_by_deadline_to_epoch_while_proving,
    advance_by_deadline_to_index, advance_to_proving_deadline, apply_ok, bf_all, create_accounts,
    create_miner, cron_tick, invariant_failure_patterns, market_add_balance, market_publish_deal,
    miner_precommit_sector, miner_prove_sector, sector_deadline, submit_windowed_post,
    verifreg_add_client, verifreg_add_verifier,
};
use test_vm::{ExpectInvocation, TestVM, VM};

#[test]
fn extend_legacy_sector_with_deals() {
    extend_legacy_sector_with_deals_inner(false);
}

#[test]
fn extend2_legacy_sector_with_deals() {
    extend_legacy_sector_with_deals_inner(true);
}

#[allow(clippy::too_many_arguments)]
fn extend<BS: Blockstore>(
    v: &TestVM<BS>,
    worker: Address,
    maddr: Address,
    deadline_index: u64,
    partition_index: u64,
    sector_number: SectorNumber,
    new_expiration: ChainEpoch,
    power_update_params: IpldBlock,
    v2: bool,
) {
    let extension_method = match v2 {
        false => MinerMethod::ExtendSectorExpiration as u64,
        true => MinerMethod::ExtendSectorExpiration2 as u64,
    };

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

    ExpectInvocation {
        to: maddr,
        method: extension_method,
        subinvocs: Some(vec![ExpectInvocation {
            to: STORAGE_POWER_ACTOR_ADDR,
            method: PowerMethod::UpdateClaimedPower as u64,
            params: Some(Some(power_update_params)),
            ..Default::default()
        }]),
        ..Default::default()
    }
    .matches(v.take_invocations().last().unwrap());
}

fn extend_legacy_sector_with_deals_inner(do_extend2: bool) {
    let store = MemoryBlockstore::new();
    let v = TestVM::<MemoryBlockstore>::new_with_singletons(&store);
    let addrs = create_accounts(&v, 3, &TokenAmount::from_whole(10_000));
    let seal_proof = RegisteredSealProof::StackedDRG32GiBV1P1;
    let (owner, worker, verifier, verified_client) = (addrs[0], addrs[0], addrs[1], addrs[2]);
    let sector_number: SectorNumber = 100;
    let policy = Policy::default();

    // create miner
    let miner_id = create_miner(
        &v,
        &owner,
        &worker,
        seal_proof.registered_window_post_proof().unwrap(),
        &TokenAmount::from_whole(1_000),
    )
    .0;
    let v = v.with_epoch(200);

    //
    // publish verified deals
    //

    // register verifier then verified client
    let datacap = StoragePower::from(32_u128 << 40);
    verifreg_add_verifier(&v, &verifier, datacap.clone());
    verifreg_add_client(&v, &verifier, &verified_client, datacap);

    // add market collateral for clients and miner
    market_add_balance(&v, &verified_client, &verified_client, &TokenAmount::from_whole(3));
    market_add_balance(&v, &worker, &miner_id, &TokenAmount::from_whole(64));

    // create 1 verified deal for total sector capacity for 6 months
    let deal_start = v.epoch() + max_prove_commit_duration(&Policy::default(), seal_proof).unwrap();
    let deals = market_publish_deal(
        &v,
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

    miner_precommit_sector(
        &v,
        &worker,
        &miner_id,
        seal_proof,
        sector_number,
        deals,
        deal_start + 180 * EPOCHS_IN_DAY,
    );

    // advance time to max seal duration and prove the sector
    advance_by_deadline_to_epoch(&v, &miner_id, deal_start);
    miner_prove_sector(&v, &worker, &miner_id, sector_number);
    // trigger cron to validate the prove commit
    cron_tick(&v);

    // inspect sector info

    let mut miner_state = v.get_state::<MinerState>(&miner_id).unwrap();
    let mut sector_info = miner_state.get_sector(&store, sector_number).unwrap().unwrap();
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
    v.mutate_state(&miner_id, |st: &mut MinerState| {
        let mut sectors = Sectors::load(&store, &st.sectors).unwrap();
        sectors.store(vec![sector_info.clone()]).unwrap();
        st.sectors = sectors.amt.flush().unwrap();
    });

    let initial_verified_deal_weight = sector_info.verified_deal_weight;
    let initial_deal_weight = sector_info.deal_weight;

    // advance to proving period and submit post
    let (deadline_info, partition_index) =
        advance_to_proving_deadline(&v, &miner_id, sector_number);

    let expected_power_delta = PowerPair {
        raw: StoragePower::from(32u64 << 30),
        qa: StoragePower::from(10 * (32u64 << 30)),
    };

    submit_windowed_post(
        &v,
        &worker,
        &miner_id,
        deadline_info,
        partition_index,
        Some(expected_power_delta),
    );

    // move forward one deadline so advanceWhileProving doesn't fail double submitting posts
    advance_by_deadline_to_index(
        &v,
        &miner_id,
        deadline_info.index + 1 % policy.wpost_period_deadlines,
    );

    // advance halfway through life and extend another 6 months
    // verified deal weight /= 2
    // power multiplier = (1/4)*10 + (3/4)*1 = 3.25
    // power delta = (10-3.25)*32GiB = 6.75*32GiB
    advance_by_deadline_to_epoch_while_proving(
        &v,
        &miner_id,
        &worker,
        sector_number,
        deal_start + 90 * EPOCHS_IN_DAY,
    );

    let new_expiration = deal_start + 2 * 180 * EPOCHS_IN_DAY;

    let mut expected_update_claimed_power_params = UpdateClaimedPowerParams {
        raw_byte_delta: StoragePower::zero(),
        quality_adjusted_delta: StoragePower::from(-675 * (32i64 << 30) / 100),
    };
    let mut expected_update_claimed_power_params_ser =
        IpldBlock::serialize_cbor(&expected_update_claimed_power_params).unwrap().unwrap();

    extend(
        &v,
        worker,
        miner_id,
        deadline_info.index,
        partition_index,
        sector_number,
        new_expiration,
        expected_update_claimed_power_params_ser,
        do_extend2,
    );

    // advance to 6 months (original expiration) and extend another 6 months
    // verified deal weight /= 2
    // power multiplier = (1/3)*3.25 + (2/3)*1 = 1.75
    // power delta = (3.25 - 1.75)*32GiB = 1.5*32GiB

    advance_by_deadline_to_epoch_while_proving(
        &v,
        &miner_id,
        &worker,
        sector_number,
        deal_start + 180 * EPOCHS_IN_DAY,
    );

    let new_expiration = deal_start + 3 * 180 * EPOCHS_IN_DAY;
    expected_update_claimed_power_params = UpdateClaimedPowerParams {
        raw_byte_delta: StoragePower::zero(),
        quality_adjusted_delta: StoragePower::from(-15 * (32i64 << 30) / 10),
    };
    expected_update_claimed_power_params_ser =
        IpldBlock::serialize_cbor(&expected_update_claimed_power_params).unwrap().unwrap();

    extend(
        &v,
        worker,
        miner_id,
        deadline_info.index,
        partition_index,
        sector_number,
        new_expiration,
        expected_update_claimed_power_params_ser,
        do_extend2,
    );

    miner_state = v.get_state::<MinerState>(&miner_id).unwrap();
    sector_info = miner_state.get_sector(&store, sector_number).unwrap().unwrap();
    assert_eq!(180 * 3 * EPOCHS_IN_DAY, sector_info.expiration - sector_info.activation);
    assert_eq!(initial_deal_weight, sector_info.deal_weight); // 0 space time, unchanged
    assert_eq!(initial_verified_deal_weight / 4, sector_info.verified_deal_weight);
    // two halvings => 1/4 initial verified deal weight

    v.expect_state_invariants(
        &[invariant_failure_patterns::REWARD_STATE_EPOCH_MISMATCH.to_owned()],
    );
}

#[test]
fn extend_updated_sector_with_claim() {
    let store = MemoryBlockstore::new();
    let v = TestVM::<MemoryBlockstore>::new_with_singletons(&store);
    let addrs = create_accounts(&v, 3, &TokenAmount::from_whole(10_000));
    let seal_proof = RegisteredSealProof::StackedDRG32GiBV1P1;
    let (owner, worker, verifier, verified_client) = (addrs[0], addrs[0], addrs[1], addrs[2]);
    let sector_number: SectorNumber = 100;
    let policy = Policy::default();

    // create miner
    let miner_id = create_miner(
        &v,
        &owner,
        &worker,
        seal_proof.registered_window_post_proof().unwrap(),
        &TokenAmount::from_whole(1_000),
    )
    .0;
    let v = v.with_epoch(200);

    //
    // Precommit, prove and PoSt empty sector (more fully tested in TestCommitPoStFlow)
    //

    let expiration = v.epoch() + 360 * EPOCHS_IN_DAY;

    miner_precommit_sector(&v, &worker, &miner_id, seal_proof, sector_number, vec![], expiration);

    // advance time by a day and prove the sector
    let prove_epoch = v.epoch() + EPOCHS_IN_DAY;
    advance_by_deadline_to_epoch(&v, &miner_id, prove_epoch);
    miner_prove_sector(&v, &worker, &miner_id, sector_number);
    // trigger cron to validate the prove commit
    cron_tick(&v);

    // advance to proving period and submit post

    let (deadline_info, partition_index) =
        advance_to_proving_deadline(&v, &miner_id, sector_number);

    let expected_power_delta =
        PowerPair { raw: StoragePower::from(32u64 << 30), qa: StoragePower::from(32u64 << 30) };

    submit_windowed_post(
        &v,
        &worker,
        &miner_id,
        deadline_info,
        partition_index,
        Some(expected_power_delta),
    );

    // move forward one deadline so sector is mutable
    advance_by_deadline_to_index(
        &v,
        &miner_id,
        deadline_info.index + 1 % policy.wpost_period_deadlines,
    );

    // Inspect basic sector info

    let miner_state = v.get_state::<MinerState>(&miner_id).unwrap();
    let initial_sector_info = miner_state.get_sector(&store, sector_number).unwrap().unwrap();
    assert_eq!(expiration, initial_sector_info.expiration);
    assert!(initial_sector_info.deal_weight.is_zero()); // 0 space time
    assert!(initial_sector_info.verified_deal_weight.is_zero()); // 0 space time
                                                                 // publish verified deals

    // register verifier then verified client
    let datacap = StoragePower::from(32_u128 << 40);
    verifreg_add_verifier(&v, &verifier, datacap.clone());
    verifreg_add_client(&v, &verifier, &verified_client, datacap);

    // add market collateral for clients and miner
    market_add_balance(&v, &verified_client, &verified_client, &TokenAmount::from_whole(3));
    market_add_balance(&v, &worker, &miner_id, &TokenAmount::from_whole(64));

    // create 1 verified deal for total sector capacity
    let deal_start = v.epoch() + EPOCHS_IN_DAY;
    let deal_ids = market_publish_deal(
        &v,
        &worker,
        &verified_client,
        &miner_id,
        "deal1".to_string(),
        PaddedPieceSize(32u64 << 30),
        true,
        deal_start,
        340 * EPOCHS_IN_DAY,
    )
    .ids;

    // replica update
    let new_cid = make_sealed_cid(b"replica1");
    let (d_idx, p_idx) = sector_deadline(&v, &miner_id, sector_number);
    let replica_update = ReplicaUpdate2 {
        sector_number,
        deadline: d_idx,
        partition: p_idx,
        new_sealed_cid: new_cid,
        deals: deal_ids.clone(),
        update_proof_type: fvm_shared::sector::RegisteredUpdateProof::StackedDRG32GiBV1,
        replica_proof: vec![],
        new_unsealed_cid: make_piece_cid(b"unsealed from itest vm"),
    };
    let updated_sectors: BitField = apply_ok(
        &v,
        &worker,
        &miner_id,
        &TokenAmount::zero(),
        MinerMethod::ProveReplicaUpdates2 as u64,
        Some(ProveReplicaUpdatesParams2 { updates: vec![replica_update] }),
    )
    .deserialize()
    .unwrap();
    assert_eq!(vec![sector_number], bf_all(updated_sectors));

    let old_power = power_for_sector(seal_proof.sector_size().unwrap(), &initial_sector_info);
    let expected_update_claimed_power_params = UpdateClaimedPowerParams {
        raw_byte_delta: StoragePower::zero(),
        quality_adjusted_delta: 9 * old_power.qa, // sector now fully qap, 10x - x = 9x
    };

    // check for the expected subcalls
    ExpectInvocation {
        to: miner_id,
        method: MinerMethod::ProveReplicaUpdates2 as u64,
        subinvocs: Some(vec![
            ExpectInvocation {
                to: STORAGE_MARKET_ACTOR_ADDR,
                method: MarketMethod::ActivateDeals as u64,
                ..Default::default()
            },
            ExpectInvocation {
                to: VERIFIED_REGISTRY_ACTOR_ADDR,
                method: VerifregMethod::ClaimAllocations as u64,
                ..Default::default()
            },
            ExpectInvocation {
                to: STORAGE_MARKET_ACTOR_ADDR,
                method: MarketMethod::VerifyDealsForActivation as u64,
                ..Default::default()
            },
            ExpectInvocation {
                to: REWARD_ACTOR_ADDR,
                method: RewardMethod::ThisEpochReward as u64,
                ..Default::default()
            },
            ExpectInvocation {
                to: STORAGE_POWER_ACTOR_ADDR,
                method: PowerMethod::CurrentTotalPower as u64,
                ..Default::default()
            },
            ExpectInvocation {
                to: STORAGE_POWER_ACTOR_ADDR,
                method: PowerMethod::UpdatePledgeTotal as u64,
                ..Default::default()
            },
            ExpectInvocation {
                to: STORAGE_POWER_ACTOR_ADDR,
                method: PowerMethod::UpdateClaimedPower as u64,
                params: Some(
                    IpldBlock::serialize_cbor(&expected_update_claimed_power_params).unwrap(),
                ),
                ..Default::default()
            },
        ]),
        ..Default::default()
    }
    .matches(v.take_invocations().last().unwrap());

    // inspect sector info

    let miner_state = v.get_state::<MinerState>(&miner_id).unwrap();
    let sector_info_after_update = miner_state.get_sector(&store, sector_number).unwrap().unwrap();
    assert_eq!(StoragePower::zero(), sector_info_after_update.deal_weight); // 0 space time

    assert_eq!(
        DealWeight::from((sector_info_after_update.expiration - v.epoch()) * (32i64 << 30)),
        sector_info_after_update.verified_deal_weight
    ); // 32 GiB * the remaining life of the sector

    // extend the updated sector

    // For clarity in checking power_base_epoch, we increment epoch by 1
    let curr_epoch = v.epoch();
    let v = v.with_epoch(curr_epoch + 1);

    let market_state: MarketState = v.get_state(&STORAGE_MARKET_ACTOR_ADDR).unwrap();
    let deal_states = DealMetaArray::load(&market_state.states, v.store).unwrap();
    let deal_state = deal_states.get(deal_ids[0]).unwrap().unwrap();
    let claim_id = deal_state.verified_claim;

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
        &v,
        &worker,
        &miner_id,
        &TokenAmount::zero(),
        MinerMethod::ExtendSectorExpiration2 as u64,
        Some(extension_params),
    );

    let miner_state = v.get_state::<MinerState>(&miner_id).unwrap();
    let sector_info_after_extension =
        miner_state.get_sector(&store, sector_number).unwrap().unwrap();
    assert_eq!(StoragePower::zero(), sector_info_after_extension.deal_weight); // 0 space time

    assert_eq!(
        DealWeight::from(
            (sector_info_after_extension.expiration - sector_info_after_update.activation)
                * (32i64 << 30)
        ),
        sector_info_after_extension.verified_deal_weight
    ); // 32 GiB * the remaining life of the sector

    assert_eq!(sector_info_after_update.activation, sector_info_after_extension.activation);
}
