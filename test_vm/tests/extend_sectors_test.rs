use fil_actor_miner::{
    max_prove_commit_duration, ExpirationExtension, ExpirationExtension2,
    ExtendSectorExpiration2Params, ExtendSectorExpirationParams, Method as MinerMethod, PowerPair,
    Sectors, State as MinerState,
};
use fil_actor_power::{Method as PowerMethod, UpdateClaimedPowerParams};
use fil_actors_runtime::runtime::Policy;
use fil_actors_runtime::{DealWeight, EPOCHS_IN_DAY, STORAGE_POWER_ACTOR_ADDR};
use fvm_ipld_bitfield::BitField;
use fvm_ipld_blockstore::MemoryBlockstore;
use fvm_ipld_encoding::ipld_block::IpldBlock;
use fvm_shared::address::Address;
use fvm_shared::bigint::Zero;
use fvm_shared::clock::ChainEpoch;
use fvm_shared::econ::TokenAmount;
use fvm_shared::piece::PaddedPieceSize;
use fvm_shared::sector::{RegisteredSealProof, SectorNumber, StoragePower};
use test_vm::util::{
    advance_by_deadline_to_epoch, advance_by_deadline_to_epoch_while_proving,
    advance_by_deadline_to_index, advance_to_proving_deadline, apply_ok, create_accounts,
    create_miner, cron_tick, invariant_failure_patterns, market_add_balance, market_publish_deal,
    miner_precommit_sector, miner_prove_sector, submit_windowed_post, verifreg_add_client,
    verifreg_add_verifier,
};
use test_vm::{ExpectInvocation, VM};

#[test]
fn extend_legacy_sector_with_deals() {
    extend_legacy_sector_with_deals_inner(false);
}

#[test]
fn extend2_legacy_sector_with_deals() {
    extend_legacy_sector_with_deals_inner(true);
}

#[allow(clippy::too_many_arguments)]
fn extend(
    v: &VM,
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
                worker,
                maddr,
                TokenAmount::zero(),
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
                worker,
                maddr,
                TokenAmount::zero(),
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
    let mut v = VM::new_with_singletons(&store);
    let addrs = create_accounts(&v, 3, TokenAmount::from_whole(10_000));
    let seal_proof = RegisteredSealProof::StackedDRG32GiBV1P1;
    let (owner, worker, verifier, verified_client) = (addrs[0], addrs[0], addrs[1], addrs[2]);
    let sector_number: SectorNumber = 100;
    let policy = Policy::default();

    // create miner
    let miner_id = create_miner(
        &mut v,
        owner,
        worker,
        seal_proof.registered_window_post_proof().unwrap(),
        TokenAmount::from_whole(1_000),
    )
    .0;
    let mut v = v.with_epoch(200);

    //
    // publish verified deals
    //

    // register verifier then verified client
    let datacap = StoragePower::from(32_u128 << 40);
    verifreg_add_verifier(&v, verifier, datacap.clone());
    verifreg_add_client(&v, verifier, verified_client, datacap);

    // add market collateral for clients and miner
    market_add_balance(&v, verified_client, verified_client, TokenAmount::from_whole(3));
    market_add_balance(&v, worker, miner_id, TokenAmount::from_whole(64));

    // create 1 verified deal for total sector capacity for 6 months
    let deal_start =
        v.get_epoch() + max_prove_commit_duration(&Policy::default(), seal_proof).unwrap();
    let deals = market_publish_deal(
        &v,
        worker,
        verified_client,
        miner_id,
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
        worker,
        miner_id,
        seal_proof,
        sector_number,
        deals,
        deal_start + 180 * EPOCHS_IN_DAY,
    );

    // advance time to max seal duration and prove the sector
    v = advance_by_deadline_to_epoch(v, miner_id, deal_start).0;
    miner_prove_sector(&v, worker, miner_id, sector_number);
    // trigger cron to validate the prove commit
    cron_tick(&v);

    // inspect sector info

    let mut miner_state = v.get_state::<MinerState>(miner_id).unwrap();
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
    v.mutate_state(miner_id, |st: &mut MinerState| {
        let mut sectors = Sectors::load(&store, &st.sectors).unwrap();
        sectors.store(vec![sector_info.clone()]).unwrap();
        st.sectors = sectors.amt.flush().unwrap();
    });

    let initial_verified_deal_weight = sector_info.verified_deal_weight;
    let initial_deal_weight = sector_info.deal_weight;

    // advance to proving period and submit post
    let (deadline_info, partition_index, mut v) =
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

    // move forward one deadline so advanceWhileProving doesn't fail double submitting posts
    v = advance_by_deadline_to_index(
        v,
        miner_id,
        deadline_info.index + 1 % policy.wpost_period_deadlines,
    )
    .0;

    // advance halfway through life and extend another 6 months
    // verified deal weight /= 2
    // power multiplier = (1/4)*10 + (3/4)*1 = 3.25
    // power delta = (10-3.25)*32GiB = 6.75*32GiB
    v = advance_by_deadline_to_epoch_while_proving(
        v,
        miner_id,
        worker,
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

    v = advance_by_deadline_to_epoch_while_proving(
        v,
        miner_id,
        worker,
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

    miner_state = v.get_state::<MinerState>(miner_id).unwrap();
    sector_info = miner_state.get_sector(&store, sector_number).unwrap().unwrap();
    assert_eq!(180 * 3 * EPOCHS_IN_DAY, sector_info.expiration - sector_info.activation);
    assert_eq!(initial_deal_weight, sector_info.deal_weight); // 0 space time, unchanged
    assert_eq!(initial_verified_deal_weight / 4, sector_info.verified_deal_weight);
    // two halvings => 1/4 initial verified deal weight

    v.expect_state_invariants(
        &[invariant_failure_patterns::REWARD_STATE_EPOCH_MISMATCH.to_owned()],
    );
}
