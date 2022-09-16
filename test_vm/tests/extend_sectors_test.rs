use fil_actor_cron::Method as CronMethod;
use fil_actor_market::Method as MarketMethod;
use fil_actor_miner::{
    max_prove_commit_duration, ExpirationExtension, ExtendSectorExpirationParams,
    Method as MinerMethod, PowerPair, PreCommitSectorParams, ProveCommitSectorParams, Sectors,
    State as MinerState,
};
use fil_actor_power::{Method as PowerMethod, UpdateClaimedPowerParams};
use fil_actor_verifreg::{AddVerifierClientParams, Method as VerifregMethod};
use fil_actors_runtime::cbor::serialize;
use fil_actors_runtime::runtime::Policy;
use fil_actors_runtime::test_utils::make_sealed_cid;
use fil_actors_runtime::{
    DealWeight, CRON_ACTOR_ADDR, EPOCHS_IN_DAY, STORAGE_MARKET_ACTOR_ADDR,
    STORAGE_POWER_ACTOR_ADDR, SYSTEM_ACTOR_ADDR, VERIFIED_REGISTRY_ACTOR_ADDR,
};
use fvm_ipld_bitfield::BitField;
use fvm_ipld_blockstore::MemoryBlockstore;
use fvm_ipld_encoding::RawBytes;
use fvm_shared::bigint::Zero;
use fvm_shared::econ::TokenAmount;
use fvm_shared::piece::PaddedPieceSize;
use fvm_shared::sector::{RegisteredSealProof, SectorNumber, StoragePower};
use test_vm::util::{
    add_verifier, advance_by_deadline_to_epoch, advance_by_deadline_to_epoch_while_proving,
    advance_by_deadline_to_index, advance_to_proving_deadline, apply_ok, create_accounts,
    create_miner, publish_deal, submit_windowed_post,
};
use test_vm::{ExpectInvocation, VM};

#[test]
fn extend_sector_with_deals() {
    let store = MemoryBlockstore::new();
    let mut v = VM::new_with_singletons(&store);
    let addrs = create_accounts(&v, 3, TokenAmount::from_whole(10_000));
    let seal_proof = RegisteredSealProof::StackedDRG32GiBV1P1;
    let (owner, worker, verifier, verified_client) = (addrs[0], addrs[0], addrs[1], addrs[2]);
    let sector_number: SectorNumber = 100;
    let sealed_cid = make_sealed_cid(b"s100");
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

    add_verifier(&v, verifier, StoragePower::from((32_u64 << 40) as u128));
    let add_client_params = AddVerifierClientParams {
        address: verified_client,
        allowance: StoragePower::from((32_u64 << 40) as u64),
    };
    apply_ok(
        &v,
        verifier,
        *VERIFIED_REGISTRY_ACTOR_ADDR,
        TokenAmount::zero(),
        VerifregMethod::AddVerifiedClient as u64,
        add_client_params,
    );

    // add market collateral for clients and miner
    let mut collateral = TokenAmount::from_whole(3);
    apply_ok(
        &v,
        verified_client,
        *STORAGE_MARKET_ACTOR_ADDR,
        collateral.clone(),
        MarketMethod::AddBalance as u64,
        verified_client,
    );
    collateral = TokenAmount::from_whole(64);
    apply_ok(
        &v,
        worker,
        *STORAGE_MARKET_ACTOR_ADDR,
        collateral,
        MarketMethod::AddBalance as u64,
        miner_id,
    );

    // create 1 verified deal for total sector capacity for 6 months
    let deal_start =
        v.get_epoch() + max_prove_commit_duration(&Policy::default(), seal_proof).unwrap();
    let deals = publish_deal(
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

    // precommit sector
    let precommit_params = PreCommitSectorParams {
        seal_proof,
        sector_number,
        sealed_cid,
        seal_rand_epoch: v.get_epoch() - 1,
        deal_ids: deals,
        expiration: deal_start + 180 * EPOCHS_IN_DAY,
        replace_capacity: false,
        replace_sector_deadline: 0,
        replace_sector_partition: 0,
        replace_sector_number: 0,
    };

    apply_ok(
        &v,
        worker,
        miner_id,
        TokenAmount::zero(),
        MinerMethod::PreCommitSector as u64,
        precommit_params,
    );

    // advance time to max seal duration

    v = advance_by_deadline_to_epoch(v, miner_id, deal_start).0;

    // Prove commit sector

    let prove_commit_params = ProveCommitSectorParams { sector_number, proof: vec![] };
    apply_ok(
        &v,
        worker,
        miner_id,
        TokenAmount::zero(),
        MinerMethod::ProveCommitSector as u64,
        prove_commit_params,
    );

    // In the same epoch, trigger cron to validate prove commit
    apply_ok(
        &v,
        *SYSTEM_ACTOR_ADDR,
        *CRON_ACTOR_ADDR,
        TokenAmount::zero(),
        CronMethod::EpochTick as u64,
        RawBytes::default(),
    );

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
    v.set_state(miner_id, |st: &mut MinerState| {
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

    let mut extension_params = ExtendSectorExpirationParams {
        extensions: vec![ExpirationExtension {
            deadline: deadline_info.index,
            partition: partition_index,
            sectors: fvm_ipld_bitfield::UnvalidatedBitField::Validated(
                BitField::try_from_bits([sector_number].iter().copied()).unwrap(),
            ),
            new_expiration: deal_start + 2 * 180 * EPOCHS_IN_DAY,
        }],
    };

    apply_ok(
        &v,
        worker,
        miner_id,
        TokenAmount::zero(),
        MinerMethod::ExtendSectorExpiration as u64,
        extension_params,
    );
    let mut expected_update_claimed_power_params = UpdateClaimedPowerParams {
        raw_byte_delta: StoragePower::zero(),
        quality_adjusted_delta: StoragePower::from(-675 * (32i64 << 30) / 100),
    };
    let mut expected_update_claimed_power_params_ser =
        serialize(&expected_update_claimed_power_params, "update_claimed_power params").unwrap();

    ExpectInvocation {
        to: miner_id,
        method: MinerMethod::ExtendSectorExpiration as u64,
        subinvocs: Some(vec![ExpectInvocation {
            to: *STORAGE_POWER_ACTOR_ADDR,
            method: PowerMethod::UpdateClaimedPower as u64,
            params: Some(expected_update_claimed_power_params_ser),
            ..Default::default()
        }]),
        ..Default::default()
    }
    .matches(v.take_invocations().last().unwrap());

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

    extension_params = ExtendSectorExpirationParams {
        extensions: vec![ExpirationExtension {
            deadline: deadline_info.index,
            partition: partition_index,
            sectors: fvm_ipld_bitfield::UnvalidatedBitField::Validated(
                BitField::try_from_bits([sector_number].iter().copied()).unwrap(),
            ),
            new_expiration: deal_start + 3 * 180 * EPOCHS_IN_DAY,
        }],
    };

    apply_ok(
        &v,
        worker,
        miner_id,
        TokenAmount::zero(),
        MinerMethod::ExtendSectorExpiration as u64,
        extension_params,
    );

    expected_update_claimed_power_params = UpdateClaimedPowerParams {
        raw_byte_delta: StoragePower::zero(),
        quality_adjusted_delta: StoragePower::from(-15 * (32i64 << 30) / 10),
    };
    expected_update_claimed_power_params_ser =
        serialize(&expected_update_claimed_power_params, "update_claimed_power params").unwrap();

    ExpectInvocation {
        to: miner_id,
        method: MinerMethod::ExtendSectorExpiration as u64,
        subinvocs: Some(vec![ExpectInvocation {
            to: *STORAGE_POWER_ACTOR_ADDR,
            method: PowerMethod::UpdateClaimedPower as u64,
            params: Some(expected_update_claimed_power_params_ser),
            ..Default::default()
        }]),
        ..Default::default()
    }
    .matches(v.take_invocations().last().unwrap());

    miner_state = v.get_state::<MinerState>(miner_id).unwrap();
    sector_info = miner_state.get_sector(&store, sector_number).unwrap().unwrap();
    assert_eq!(180 * 3 * EPOCHS_IN_DAY, sector_info.expiration - sector_info.activation);
    assert_eq!(initial_deal_weight, sector_info.deal_weight); // 0 space time, unchanged
    assert_eq!(initial_verified_deal_weight / 4, sector_info.verified_deal_weight);
    // two halvings => 1/4 initial verified deal weight
}
