use export_macro::vm_test;
use fil_actor_miner::ExpirationQueue;
use fil_actor_miner::{
    ExpirationExtension2, ExtendSectorExpiration2Params, Method as MinerMethod, PowerPair,
    ProveCommitSectorsNIParams, ProveCommitSectorsNIReturn, ProveReplicaUpdates3Params,
    ProveReplicaUpdates3Return, SectorNIActivationInfo, SectorOnChainInfoFlags,
    SectorUpdateManifest, Sectors, State as MinerState, TerminateSectorsParams,
    TerminationDeclaration, daily_proof_fee, daily_proof_fee_adjust, initial_pledge_for_power,
    max_prove_commit_duration, qa_power_for_sector,
};
use fil_actor_multisig::Method as MultisigMethod;
use fil_actor_power::{Method as PowerMethod, State as PowerState};
use fil_actor_reward::State as RewardState;
use fil_actor_verifreg::{AddVerifiedClientParams, Method as VerifregMethod, VerifierParams};
use fil_actors_runtime::DealWeight;
use fil_actors_runtime::cbor::serialize;
use fil_actors_runtime::runtime::Policy;
use fil_actors_runtime::test_utils::make_sealed_cid;
use fil_actors_runtime::{
    EPOCHS_IN_DAY, REWARD_ACTOR_ADDR, STORAGE_POWER_ACTOR_ADDR, VERIFIED_REGISTRY_ACTOR_ADDR,
};
use fvm_ipld_encoding::RawBytes;
use fvm_shared::address::Address;
use fvm_shared::bigint::BigInt;
use fvm_shared::econ::TokenAmount;
use fvm_shared::error::ExitCode;
use fvm_shared::piece::PaddedPieceSize;
use fvm_shared::sector::{
    RegisteredAggregateProof, RegisteredSealProof, SectorNumber, SectorSize, StoragePower,
};
use num_traits::{Signed, Zero};

use fil_actor_multisig::ProposeParams;
use fil_actor_verifreg::State as VerifregState;
use vm_api::VM;
use vm_api::util::{DynBlockstore, apply_code, apply_ok, get_state, mutate_state};

use crate::tests::{create_deals, create_sector};
use crate::util::make_bitfield;
use crate::util::{
    PrecommitMetadata, advance_by_deadline_to_epoch, advance_by_deadline_to_epoch_while_proving,
    advance_by_deadline_to_index, advance_to_proving_deadline, assert_invariants, create_accounts,
    create_miner, cron_tick, datacap_get_balance, get_network_stats,
    make_piece_manifests_from_deal_ids, market_add_balance, market_publish_deal, miner_power,
    miner_precommit_one_sector_v2, miner_prove_sector, override_compute_unsealed_sector_cid,
    precommit_meta_data_from_deals, sector_info, submit_windowed_post,
};

/// A new CC sector committed via ProveCommitSectors3 gets 10x QA power.
#[vm_test]
pub fn new_cc_sector_gets_10x_test(v: &dyn VM) {
    override_compute_unsealed_sector_cid(v);
    let policy = Policy::default();
    let addrs = create_accounts(v, 1, &TokenAmount::from_whole(10_000));
    let seal_proof = RegisteredSealProof::StackedDRG32GiBV1P1;
    let sector_size = seal_proof.sector_size().unwrap() as u64;
    let (owner, worker) = (addrs[0], addrs[0]);
    let (maddr, _) = create_miner(
        v,
        &owner,
        &worker,
        seal_proof.registered_window_post_proof().unwrap(),
        &TokenAmount::from_whole(8_000),
    );

    v.set_epoch(200);

    let sector_number: SectorNumber = 100;
    let expiration = v.epoch()
        + policy.min_sector_expiration
        + max_prove_commit_duration(&policy, seal_proof).unwrap()
        + 1;

    // Precommit a CC sector (no deals)
    miner_precommit_one_sector_v2(
        v,
        &worker,
        &maddr,
        seal_proof,
        sector_number,
        PrecommitMetadata::default(),
        true,
        expiration,
    );

    // Advance to prove commit
    let prove_time = v.epoch() + policy.pre_commit_challenge_delay + 1;
    advance_by_deadline_to_epoch(v, &maddr, prove_time);

    // Prove commit via ProveCommitSectors3 (no pieces = CC sector)
    miner_prove_sector(v, &worker, &maddr, sector_number, vec![]);
    cron_tick(v);

    // Verify sector has FULL_QA_POWER flag
    let si = sector_info(v, &maddr, sector_number);
    assert!(
        si.flags.contains(SectorOnChainInfoFlags::FULL_QA_POWER),
        "CC sector should have FULL_QA_POWER flag set"
    );
    assert!(
        si.flags.contains(SectorOnChainInfoFlags::SIMPLE_QA_POWER),
        "CC sector should have SIMPLE_QA_POWER flag set"
    );
    // A CC sector holds no pieces, so it reaches 10x on the flag alone with both weights zero.
    assert!(si.deal_weight.is_zero(), "CC sector should have no deal weight");
    assert!(si.verified_deal_weight.is_zero(), "CC sector should have no verified deal weight");

    // Advance to proving deadline and submit Window PoSt
    let (deadline_info, partition_index) = advance_to_proving_deadline(v, &maddr, sector_number);

    // Expected power: raw = sector_size, qa = 10 * sector_size (FULL_QA_POWER)
    let expected_power = fil_actor_miner::PowerPair {
        raw: StoragePower::from(sector_size),
        qa: StoragePower::from(10 * sector_size),
    };
    submit_windowed_post(v, &worker, &maddr, deadline_info, partition_index, Some(expected_power));

    // Advance past deadline to activate power
    advance_by_deadline_to_index(
        v,
        &maddr,
        (deadline_info.index + 1) % policy.wpost_period_deadlines,
    );

    // Verify power claim from power actor: QA power == 10x raw power
    let power = miner_power(v, &maddr);
    assert_eq!(power.raw, BigInt::from(sector_size), "Raw power should be sector_size");
    assert_eq!(
        power.qa,
        BigInt::from(10 * sector_size),
        "QA power should be 10x raw power for CC sector"
    );

    assert_invariants(v, &Policy::default(), None);
}

/// A sector committed via ProveCommitSectorsNI gets 10x QA power.
#[vm_test]
pub fn ni_sector_gets_10x_test(v: &dyn VM) {
    let policy = Policy::default();
    let addrs = create_accounts(v, 1, &TokenAmount::from_whole(10_000));
    let seal_proof = RegisteredSealProof::StackedDRG32GiBV1P2_Feat_NiPoRep;
    let sector_size = seal_proof.sector_size().unwrap() as u64;
    let (owner, worker) = (addrs[0], addrs[0]);
    let (maddr, _) = create_miner(
        v,
        &owner,
        &worker,
        seal_proof.registered_window_post_proof().unwrap(),
        &TokenAmount::from_whole(8_000),
    );
    let miner_id = maddr.id().unwrap();

    // Onboard a single sector via NI-PoRep
    let seal_rand_epoch = v.epoch();
    let activation_epoch = seal_rand_epoch + policy.max_prove_commit_ni_randomness_lookback / 2;
    let expiration = activation_epoch + policy.min_sector_expiration + 1;
    let sector_number: SectorNumber = 100;
    let proving_deadline = 7;

    let ni_sector_info = SectorNIActivationInfo {
        sealing_number: sector_number,
        sealer_id: miner_id,
        sector_number,
        sealed_cid: make_sealed_cid(format!("sn: {}", sector_number).as_bytes()),
        seal_rand_epoch,
        expiration,
    };

    let params = ProveCommitSectorsNIParams {
        sectors: vec![ni_sector_info],
        seal_proof_type: seal_proof,
        aggregate_proof: RawBytes::new(vec![1, 2, 3, 4]),
        aggregate_proof_type: RegisteredAggregateProof::SnarkPackV2,
        proving_deadline,
        require_activation_success: true,
    };

    v.set_epoch(activation_epoch);

    let ret: ProveCommitSectorsNIReturn = apply_ok(
        v,
        &worker,
        &maddr,
        &TokenAmount::zero(),
        MinerMethod::ProveCommitSectorsNI as u64,
        Some(params),
    )
    .deserialize()
    .unwrap();
    assert!(ret.activation_results.all_ok());

    // Verify sector has FULL_QA_POWER flag
    let si = sector_info(v, &maddr, sector_number);
    assert!(
        si.flags.contains(SectorOnChainInfoFlags::FULL_QA_POWER),
        "NI sector should have FULL_QA_POWER flag set"
    );
    assert!(
        si.flags.contains(SectorOnChainInfoFlags::SIMPLE_QA_POWER),
        "NI sector should have SIMPLE_QA_POWER flag set"
    );
    // NI sectors carry no data, so neither weight is populated.
    assert!(si.deal_weight.is_zero(), "NI sector should have no deal weight");
    assert!(si.verified_deal_weight.is_zero(), "NI sector should have no verified deal weight");

    // Advance to proving deadline, submit Window PoSt
    let deadline_info = advance_by_deadline_to_index(v, &maddr, proving_deadline);

    let store = &DynBlockstore::wrap(v.blockstore());
    let deadline = crate::util::deadline_state(v, &maddr, proving_deadline);
    let partition = deadline.load_partition(store, 0).unwrap();

    submit_windowed_post(v, &worker, &maddr, deadline_info, 0, Some(partition.unproven_power));

    // Advance past deadline to activate power
    advance_by_deadline_to_index(v, &maddr, (proving_deadline + 1) % policy.wpost_period_deadlines);

    // Verify 10x QA power in power actor
    let power = miner_power(v, &maddr);
    assert_eq!(power.raw, BigInt::from(sector_size), "Raw power should be sector_size");
    assert_eq!(
        power.qa,
        BigInt::from(10 * sector_size),
        "QA power should be 10x raw power for NI sector"
    );

    assert_invariants(v, &Policy::default(), None);
}

/// AddVerifier and AddVerifiedClient on the verifreg actor are rejected.
#[vm_test]
pub fn verifreg_minting_disabled_test(v: &dyn VM) {
    let addrs = create_accounts(v, 2, &TokenAmount::from_whole(10_000));
    let (verifier, client) = (addrs[0], addrs[1]);
    let verifier_allowance = StoragePower::from(2 * 1048576u64);

    // Try to call AddVerifier via the verifreg root msig - should fail with USR_FORBIDDEN
    let add_verifier_params =
        VerifierParams { address: verifier, allowance: verifier_allowance.clone() };
    let proposal = ProposeParams {
        to: VERIFIED_REGISTRY_ACTOR_ADDR,
        value: TokenAmount::zero(),
        method: VerifregMethod::AddVerifier as u64,
        params: serialize(&add_verifier_params, "verifreg add verifier params").unwrap(),
    };

    // The multisig Propose itself succeeds, but the inner call to AddVerifier returns USR_FORBIDDEN
    apply_ok(
        v,
        &crate::TEST_VERIFREG_ROOT_SIGNER_ADDR,
        &crate::TEST_VERIFREG_ROOT_ADDR,
        &TokenAmount::zero(),
        MultisigMethod::Propose as u64,
        Some(proposal),
    );

    // Verify that the verifier was NOT added
    let v_st: VerifregState = get_state(v, &VERIFIED_REGISTRY_ACTOR_ADDR).unwrap();
    let store = DynBlockstore::wrap(v.blockstore());
    let cap = v_st.get_verifier_cap(&store, &verifier).unwrap();
    assert!(cap.is_none(), "Verifier should not have been added (AddVerifier is deprecated)");

    // Try to call AddVerifiedClient directly - should fail with USR_FORBIDDEN
    let add_client_params =
        AddVerifiedClientParams { address: client, allowance: verifier_allowance };
    apply_code(
        v,
        &verifier,
        &VERIFIED_REGISTRY_ACTOR_ADDR,
        &TokenAmount::zero(),
        VerifregMethod::AddVerifiedClient as u64,
        Some(add_client_params),
        ExitCode::USR_FORBIDDEN,
    );

    assert_invariants(v, &Policy::default(), None);
}

/// Publishing a verified deal transfers no datacap, and the sector reaches 10x like any
/// other, recording its piece spacetime in `verified_deal_weight`.
#[vm_test]
pub fn verified_deal_gets_10x_without_datacap_test(v: &dyn VM) {
    override_compute_unsealed_sector_cid(v);
    let policy = Policy::default();
    let addrs = create_accounts(v, 3, &TokenAmount::from_whole(10_000));
    let seal_proof = RegisteredSealProof::StackedDRG32GiBV1P1;
    let sector_size = seal_proof.sector_size().unwrap() as u64;
    let (owner, worker, client) = (addrs[0], addrs[0], addrs[1]);
    let (maddr, _) = create_miner(
        v,
        &owner,
        &worker,
        seal_proof.registered_window_post_proof().unwrap(),
        &TokenAmount::from_whole(8_000),
    );

    v.set_epoch(200);

    // Record the client's datacap balance before publishing
    let client_datacap_before = datacap_get_balance(v, &client);

    // Add market balances for client and provider
    market_add_balance(v, &client, &client, &TokenAmount::from_whole(3));
    market_add_balance(v, &worker, &maddr, &TokenAmount::from_whole(64));

    // Publish a deal with verified_deal = true
    let deal_start = v.epoch() + max_prove_commit_duration(&Policy::default(), seal_proof).unwrap();
    let deal_lifetime = 180 * EPOCHS_IN_DAY;
    let deal_ret = market_publish_deal(
        v,
        &worker,
        &client,
        &maddr,
        "fip0118-verified-deal".to_string(),
        PaddedPieceSize(32u64 << 30),
        true, // verified_deal = true
        deal_start,
        deal_lifetime,
    );

    // Verify the deal was published successfully
    assert!(!deal_ret.ids.is_empty(), "Deal should have been published");
    let deal_id = deal_ret.ids[0];

    // Verify NO datacap tokens were transferred (client balance unchanged)
    let client_datacap_after = datacap_get_balance(v, &client);
    assert_eq!(
        client_datacap_before, client_datacap_after,
        "Client datacap balance should be unchanged (no datacap transfer for verified deals)"
    );

    // Commit a sector with this deal
    let sector_number: SectorNumber = 100;
    let deals = vec![deal_id];
    miner_precommit_one_sector_v2(
        v,
        &worker,
        &maddr,
        seal_proof,
        sector_number,
        precommit_meta_data_from_deals(v, &deals, seal_proof, false),
        true,
        deal_start + deal_lifetime,
    );

    // Advance to prove commit
    advance_by_deadline_to_epoch(v, &maddr, deal_start);
    miner_prove_sector(
        v,
        &worker,
        &maddr,
        sector_number,
        make_piece_manifests_from_deal_ids(v, deals),
    );
    cron_tick(v);

    // Verify sector has FULL_QA_POWER flag
    let si = sector_info(v, &maddr, sector_number);
    assert!(
        si.flags.contains(SectorOnChainInfoFlags::FULL_QA_POWER),
        "Sector with verified deal should have FULL_QA_POWER flag"
    );
    // Piece spacetime lands in verified_deal_weight; deal_weight is unused.
    assert!(si.deal_weight.is_zero(), "deal_weight should be unused");
    assert_eq!(
        DealWeight::from(sector_size) * (si.expiration - si.power_base_epoch),
        si.verified_deal_weight,
        "verified_deal_weight should hold the full sector's piece spacetime"
    );

    // Advance to proving deadline and submit PoSt
    let (deadline_info, partition_index) = advance_to_proving_deadline(v, &maddr, sector_number);

    // Every sector reaches 10x regardless of deal content.
    let expected_power = fil_actor_miner::PowerPair {
        raw: StoragePower::from(sector_size),
        qa: StoragePower::from(10 * sector_size),
    };
    submit_windowed_post(v, &worker, &maddr, deadline_info, partition_index, Some(expected_power));

    // Advance past deadline to activate power
    advance_by_deadline_to_index(
        v,
        &maddr,
        (deadline_info.index + 1) % policy.wpost_period_deadlines,
    );

    // Verify sector gets 10x QA power (same as CC)
    let power = miner_power(v, &maddr);
    assert_eq!(power.raw, BigInt::from(sector_size), "Raw power should be sector_size");
    assert_eq!(
        power.qa,
        BigInt::from(10 * sector_size),
        "QA power should be 10x raw power, as for a CC sector"
    );

    assert_invariants(v, &Policy::default(), None);
}

/// Rewrites an active sector into an older vintage by clearing `FULL_QA_POWER`, and
/// `SIMPLE_QA_POWER` too when `simple_qap` is false, so its quality derives from weights. The
/// sector was committed at 10x, so the partition, deadline and power-actor records are reduced
/// to match, leaving state consistent.
fn make_sector_legacy(
    v: &dyn VM,
    maddr: &Address,
    sector_number: SectorNumber,
    d_idx: u64,
    p_idx: u64,
    sector_size: SectorSize,
    simple_qap: bool,
) {
    let store = &DynBlockstore::wrap(v.blockstore());
    let mut si = sector_info(v, maddr, sector_number);
    let before = qa_power_for_sector(sector_size, &si);
    si.flags.set(SectorOnChainInfoFlags::SIMPLE_QA_POWER, simple_qap);
    si.flags.set(SectorOnChainInfoFlags::FULL_QA_POWER, false);
    // A sector this old predates FIP-0100, so it carries no recorded fee; the partition and
    // deadline fee records are cleared alongside it.
    si.daily_fee = TokenAmount::zero();
    let after = qa_power_for_sector(sector_size, &si);
    let drop = PowerPair { raw: StoragePower::zero(), qa: before - after };

    mutate_state(v, maddr, |st: &mut MinerState| {
        let mut sectors = Sectors::load(store, &st.sectors).unwrap();
        sectors.store(vec![si.clone()]).unwrap();
        st.sectors = sectors.amt.flush().unwrap();

        let mut deadlines = st.load_deadlines(store).unwrap();
        let mut deadline = deadlines.load_deadline(store, d_idx).unwrap();
        let mut partition = deadline.load_partition(store, p_idx).unwrap();
        partition.live_power -= &drop;
        // The expiration queue records the same power against the sector's expiry epoch.
        let quant = st.quant_spec_for_deadline(&Policy::default(), d_idx);
        let mut queue = ExpirationQueue::new(store, &partition.expirations_epochs, quant).unwrap();
        let epoch = quant.quantize_up(si.expiration);
        let mut set = queue.amt.get(epoch as u64).unwrap().unwrap().clone();
        set.active_power -= &drop;
        set.fee_deduction = TokenAmount::zero();
        queue.amt.set(epoch as u64, set).unwrap();
        partition.expirations_epochs = queue.amt.flush().unwrap();

        let mut parts = deadline.partitions_amt(store).unwrap();
        parts.set(p_idx, partition).unwrap();
        deadline.partitions = parts.flush().unwrap();
        deadline.live_power -= &drop;
        deadline.daily_fee = TokenAmount::zero();
        deadlines.update_deadline(&Policy::default(), store, d_idx, &deadline).unwrap();
        st.save_deadlines(store, deadlines).unwrap();
    });

    mutate_state(v, &STORAGE_POWER_ACTOR_ADDR, |st: &mut PowerState| {
        let mut claims = st.load_claims(store).unwrap();
        let mut claim = claims.get(maddr).unwrap().unwrap().clone();
        claim.quality_adj_power -= &drop.qa;
        claims.set(maddr, claim).unwrap();
        st.claims = claims.flush().unwrap();
        // Claimed totals are left alone: a 32GiB miner is below the consensus minimum so its
        // power was never counted there. Committed bytes are tracked regardless.
        st.total_qa_bytes_committed -= &drop.qa;
    });
}

fn set_sector_daily_fee(
    v: &dyn VM,
    maddr: &Address,
    sector_number: SectorNumber,
    d_idx: u64,
    p_idx: u64,
    fee: &TokenAmount,
) {
    let store = &DynBlockstore::wrap(v.blockstore());
    let mut si = sector_info(v, maddr, sector_number);
    si.daily_fee = fee.clone();

    mutate_state(v, maddr, |st: &mut MinerState| {
        let mut sectors = Sectors::load(store, &st.sectors).unwrap();
        sectors.store(vec![si.clone()]).unwrap();
        st.sectors = sectors.amt.flush().unwrap();

        let mut deadlines = st.load_deadlines(store).unwrap();
        let mut deadline = deadlines.load_deadline(store, d_idx).unwrap();
        let mut partition = deadline.load_partition(store, p_idx).unwrap();
        let quant = st.quant_spec_for_deadline(&Policy::default(), d_idx);
        let mut queue = ExpirationQueue::new(store, &partition.expirations_epochs, quant).unwrap();
        let epoch = quant.quantize_up(si.expiration);
        let mut set = queue.amt.get(epoch as u64).unwrap().unwrap().clone();
        set.fee_deduction = fee.clone();
        queue.amt.set(epoch as u64, set).unwrap();
        partition.expirations_epochs = queue.amt.flush().unwrap();

        let mut parts = deadline.partitions_amt(store).unwrap();
        parts.set(p_idx, partition).unwrap();
        deadline.partitions = parts.flush().unwrap();
        deadline.daily_fee = fee.clone();
        deadlines.update_deadline(&Policy::default(), store, d_idx, &deadline).unwrap();
        st.save_deadlines(store, deadlines).unwrap();
    });
}

fn current_initial_pledge(v: &dyn VM, qa_power: &StoragePower) -> TokenAmount {
    let power: PowerState = get_state(v, &STORAGE_POWER_ACTOR_ADDR).unwrap();
    let reward: RewardState = get_state(v, &REWARD_ACTOR_ADDR).unwrap();
    initial_pledge_for_power(
        qa_power,
        &reward.this_epoch_baseline_power,
        &reward.this_epoch_reward_smoothed,
        &power.this_epoch_qa_power_smoothed,
        &v.circulating_supply(),
        v.epoch() - power.ramp_start_epoch,
        power.ramp_duration_epochs,
    )
}

fn snap_sector(
    v: &dyn VM,
    worker: Address,
    maddr: Address,
    sector_number: SectorNumber,
    d_idx: u64,
    p_idx: u64,
    seal_proof: RegisteredSealProof,
) {
    let deal_ids = create_deals(1, v, worker, worker, maddr);
    let params = ProveReplicaUpdates3Params {
        sector_updates: vec![SectorUpdateManifest {
            sector: sector_number,
            deadline: d_idx,
            partition: p_idx,
            new_sealed_cid: make_sealed_cid(b"replica-legacy"),
            pieces: make_piece_manifests_from_deal_ids(v, deal_ids),
        }],
        sector_proofs: vec![RawBytes::new(vec![1, 2, 3, 4])],
        aggregate_proof: RawBytes::default(),
        update_proofs_type: seal_proof.registered_update_proof().unwrap(),
        aggregate_proof_type: None,
        require_activation_success: true,
        require_notification_success: true,
    };
    let ret: ProveReplicaUpdates3Return = apply_ok(
        v,
        &worker,
        &maddr,
        &TokenAmount::zero(),
        MinerMethod::ProveReplicaUpdates3 as u64,
        Some(params),
    )
    .deserialize()
    .unwrap();
    assert!(ret.activation_results.all_ok());
}

/// Snapping data into a full-power sector preserves its pledge even when current network
/// conditions would calculate a higher one.
#[vm_test]
pub fn full_qa_snap_preserves_initial_pledge_test(v: &dyn VM) {
    let addrs = create_accounts(v, 1, &TokenAmount::from_whole(100_000));
    let (worker, owner) = (addrs[0], addrs[0]);
    let seal_proof = RegisteredSealProof::StackedDRG32GiBV1P1;
    let sector_size = seal_proof.sector_size().unwrap();
    let (maddr, _) = create_miner(
        v,
        &owner,
        &worker,
        seal_proof.registered_window_post_proof().unwrap(),
        &TokenAmount::from_whole(10_000),
    );

    v.set_epoch(200);
    let normal_supply = v.circulating_supply();
    let normal_reward_estimate =
        get_state::<RewardState>(v, &REWARD_ACTOR_ADDR).unwrap().this_epoch_reward_smoothed;
    v.set_circulating_supply(TokenAmount::zero());
    mutate_state(v, &REWARD_ACTOR_ADDR, |st: &mut RewardState| {
        st.this_epoch_reward_smoothed = Default::default();
    });
    let sector_number = 100;
    let (d_idx, p_idx) = create_sector(v, worker, maddr, sector_number, seal_proof);

    let sector_before = sector_info(v, &maddr, sector_number);
    let power_before = miner_power(v, &maddr);
    let state_before: MinerState = get_state(v, &maddr).unwrap();
    let network_before = get_network_stats(v);
    assert!(sector_before.flags.contains(SectorOnChainInfoFlags::FULL_QA_POWER));
    assert_eq!(BigInt::from(10 * sector_size as u64), power_before.qa);

    v.set_circulating_supply(normal_supply);
    mutate_state(v, &REWARD_ACTOR_ADDR, |st: &mut RewardState| {
        st.this_epoch_reward_smoothed = normal_reward_estimate;
    });
    let recalculated_pledge = current_initial_pledge(v, &power_before.qa);
    assert!(
        recalculated_pledge > sector_before.initial_pledge,
        "network conditions must support a pledge ratchet"
    );

    snap_sector(v, worker, maddr, sector_number, d_idx, p_idx, seal_proof);

    let sector_after = sector_info(v, &maddr, sector_number);
    let state_after: MinerState = get_state(v, &maddr).unwrap();
    let network_after = get_network_stats(v);
    assert_eq!(power_before, miner_power(v, &maddr));
    assert_eq!(sector_before.initial_pledge, sector_after.initial_pledge);
    assert_eq!(state_before.initial_pledge, state_after.initial_pledge);
    assert_eq!(network_before.total_pledge_collateral, network_after.total_pledge_collateral);

    let invocation = v.take_invocations().pop().unwrap();
    assert_eq!(MinerMethod::ProveReplicaUpdates3 as u64, invocation.method);
    assert!(
        invocation.subinvocations.iter().all(|sub| {
            sub.to != STORAGE_POWER_ACTOR_ADDR
                || sub.method != PowerMethod::UpdatePledgeTotal as u64
        }),
        "unchanged pledge must not be reported to the power actor"
    );
    assert_invariants(v, &Policy::default(), None);
}

/// Snapping a legacy sector to full power recalculates its pledge and locks the increase.
#[vm_test]
pub fn legacy_cc_snap_increases_initial_pledge_test(v: &dyn VM) {
    let addrs = create_accounts(v, 1, &TokenAmount::from_whole(100_000));
    let (worker, owner) = (addrs[0], addrs[0]);
    let seal_proof = RegisteredSealProof::StackedDRG32GiBV1P1;
    let sector_size = seal_proof.sector_size().unwrap();
    let (maddr, _) = create_miner(
        v,
        &owner,
        &worker,
        seal_proof.registered_window_post_proof().unwrap(),
        &TokenAmount::from_whole(10_000),
    );

    v.set_epoch(200);
    let normal_supply = v.circulating_supply();
    let normal_reward_estimate =
        get_state::<RewardState>(v, &REWARD_ACTOR_ADDR).unwrap().this_epoch_reward_smoothed;
    v.set_circulating_supply(TokenAmount::zero());
    mutate_state(v, &REWARD_ACTOR_ADDR, |st: &mut RewardState| {
        st.this_epoch_reward_smoothed = Default::default();
    });
    let sector_number = 100;
    let (d_idx, p_idx) = create_sector(v, worker, maddr, sector_number, seal_proof);
    make_sector_legacy(v, &maddr, sector_number, d_idx, p_idx, sector_size, false);

    let sector_before = sector_info(v, &maddr, sector_number);
    let power_before = miner_power(v, &maddr);
    let state_before: MinerState = get_state(v, &maddr).unwrap();
    let network_before = get_network_stats(v);
    assert!(!sector_before.flags.contains(SectorOnChainInfoFlags::FULL_QA_POWER));
    assert_eq!(BigInt::from(sector_size as u64), power_before.qa);

    v.set_circulating_supply(normal_supply);
    mutate_state(v, &REWARD_ACTOR_ADDR, |st: &mut RewardState| {
        st.this_epoch_reward_smoothed = normal_reward_estimate;
    });
    let expected_qa = BigInt::from(10 * sector_size as u64);
    let expected_pledge = current_initial_pledge(v, &expected_qa);
    assert!(
        expected_pledge > sector_before.initial_pledge,
        "network conditions must support a pledge ratchet"
    );

    snap_sector(v, worker, maddr, sector_number, d_idx, p_idx, seal_proof);

    let sector_after = sector_info(v, &maddr, sector_number);
    let power_after = miner_power(v, &maddr);
    let state_after: MinerState = get_state(v, &maddr).unwrap();
    let network_after = get_network_stats(v);
    let pledge_delta = &expected_pledge - &sector_before.initial_pledge;
    assert_eq!(expected_qa, power_after.qa);
    assert_eq!(BigInt::from(9 * sector_size as u64), &power_after.qa - &power_before.qa);
    assert_eq!(expected_pledge, sector_after.initial_pledge);
    assert_eq!(pledge_delta, &state_after.initial_pledge - &state_before.initial_pledge);
    assert_eq!(
        pledge_delta,
        &network_after.total_pledge_collateral - &network_before.total_pledge_collateral
    );

    let invocation = v.take_invocations().pop().unwrap();
    assert_eq!(MinerMethod::ProveReplicaUpdates3 as u64, invocation.method);
    assert!(
        invocation.subinvocations.iter().any(|sub| {
            sub.to == STORAGE_POWER_ACTOR_ADDR
                && sub.method == PowerMethod::UpdatePledgeTotal as u64
        }),
        "pledge increase must be reported to the power actor"
    );
    assert_invariants(v, &Policy::default(), None);
}

/// Snapping a CC sector that carries neither quality flag raises it to 10x QA power, with no
/// explicit upgrade call.
#[vm_test]
pub fn legacy_cc_sector_reaches_10x_by_snapping_test(v: &dyn VM) {
    let addrs = create_accounts(v, 1, &TokenAmount::from_whole(100_000));
    let (worker, owner) = (addrs[0], addrs[0]);
    let seal_proof = RegisteredSealProof::StackedDRG32GiBV1P1;
    let sector_size = seal_proof.sector_size().unwrap();
    let (maddr, _) = create_miner(
        v,
        &owner,
        &worker,
        seal_proof.registered_window_post_proof().unwrap(),
        &TokenAmount::from_whole(10_000),
    );

    v.set_epoch(200);
    let sector_number = 100;
    let (d_idx, p_idx) = create_sector(v, worker, maddr, sector_number, seal_proof);

    make_sector_legacy(v, &maddr, sector_number, d_idx, p_idx, sector_size, false);

    // A legacy CC sector sits at 1x: no flag, and no weight to derive quality from.
    let legacy = sector_info(v, &maddr, sector_number);
    assert!(!legacy.flags.contains(SectorOnChainInfoFlags::FULL_QA_POWER));
    assert_eq!(BigInt::from(sector_size as u64), qa_power_for_sector(sector_size, &legacy));
    assert_eq!(BigInt::from(sector_size as u64), miner_power(v, &maddr).qa);
    assert_invariants(v, &Policy::default(), None);

    // Snap it.
    snap_sector(v, worker, maddr, sector_number, d_idx, p_idx, seal_proof);

    let updated = sector_info(v, &maddr, sector_number);
    assert!(
        updated.flags.contains(SectorOnChainInfoFlags::FULL_QA_POWER),
        "snapping a legacy CC sector should set FULL_QA_POWER"
    );
    assert_eq!(BigInt::from(10 * sector_size as u64), qa_power_for_sector(sector_size, &updated));
    assert_eq!(BigInt::from(10 * sector_size as u64), miner_power(v, &maddr).qa);

    // Carrying no recorded fee, it is assigned a fresh one at its new 10x power rather than
    // scaled from an old one.
    assert!(legacy.daily_fee.is_zero());
    assert_eq!(
        daily_proof_fee(
            &Policy::default(),
            &v.circulating_supply(),
            &BigInt::from(10 * sector_size as u64)
        ),
        updated.daily_fee
    );

    assert_invariants(v, &Policy::default(), None);
}

/// FIP-0118 leaves existing sector flags untouched, so a fee-bearing legacy CC sector remains
/// at 1x until an upgrade. Snapping it to 10x scales every copy of its daily fee.
#[vm_test]
pub fn legacy_cc_snap_scales_existing_daily_fee_test(v: &dyn VM) {
    let addrs = create_accounts(v, 1, &TokenAmount::from_whole(100_000));
    let (worker, owner) = (addrs[0], addrs[0]);
    let seal_proof = RegisteredSealProof::StackedDRG32GiBV1P1;
    let sector_size = seal_proof.sector_size().unwrap();
    let (maddr, _) = create_miner(
        v,
        &owner,
        &worker,
        seal_proof.registered_window_post_proof().unwrap(),
        &TokenAmount::from_whole(10_000),
    );

    v.set_epoch(200);
    let sector_number = 100;
    let (d_idx, p_idx) = create_sector(v, worker, maddr, sector_number, seal_proof);
    make_sector_legacy(v, &maddr, sector_number, d_idx, p_idx, sector_size, false);

    let legacy_fee = TokenAmount::from_atto(123);
    set_sector_daily_fee(v, &maddr, sector_number, d_idx, p_idx, &legacy_fee);
    let legacy = sector_info(v, &maddr, sector_number);
    let power_before = miner_power(v, &maddr);
    assert_eq!(legacy_fee, legacy.daily_fee);
    assert_eq!(BigInt::from(sector_size as u64), power_before.qa);
    assert_invariants(v, &Policy::default(), None);

    snap_sector(v, worker, maddr, sector_number, d_idx, p_idx, seal_proof);

    let updated = sector_info(v, &maddr, sector_number);
    let power_after = miner_power(v, &maddr);
    let expected_qa = BigInt::from(10 * sector_size as u64);
    let expected_fee = daily_proof_fee_adjust(&legacy_fee, &power_before.qa, &expected_qa);
    assert!(updated.flags.contains(SectorOnChainInfoFlags::FULL_QA_POWER));
    assert_eq!(expected_qa, power_after.qa);
    assert_eq!(BigInt::from(9 * sector_size as u64), &power_after.qa - &power_before.qa);
    assert_eq!(expected_fee, updated.daily_fee);

    let store = &DynBlockstore::wrap(v.blockstore());
    let st: MinerState = get_state(v, &maddr).unwrap();
    let deadline = st.load_deadlines(store).unwrap().load_deadline(store, d_idx).unwrap();
    assert_eq!(expected_fee, deadline.daily_fee);
    let partition = deadline.load_partition(store, p_idx).unwrap();
    let quant = st.quant_spec_for_deadline(&Policy::default(), d_idx);
    let queue = ExpirationQueue::new(store, &partition.expirations_epochs, quant).unwrap();
    let set = queue.amt.get(quant.quantize_up(updated.expiration) as u64).unwrap().unwrap();
    assert_eq!(expected_fee, set.fee_deduction);

    assert_invariants(v, &Policy::default(), None);
}

/// A CC sector carrying neither quality flag holds 1x, and termination removes exactly that.
#[vm_test]
pub fn legacy_sector_termination_removes_its_own_power_test(v: &dyn VM) {
    let addrs = create_accounts(v, 1, &TokenAmount::from_whole(100_000));
    let (worker, owner) = (addrs[0], addrs[0]);
    let seal_proof = RegisteredSealProof::StackedDRG32GiBV1P1;
    let sector_size = seal_proof.sector_size().unwrap();
    let (maddr, _) = create_miner(
        v,
        &owner,
        &worker,
        seal_proof.registered_window_post_proof().unwrap(),
        &TokenAmount::from_whole(10_000),
    );

    v.set_epoch(200);
    let sector_number = 100;
    let (d_idx, p_idx) = create_sector(v, worker, maddr, sector_number, seal_proof);
    make_sector_legacy(v, &maddr, sector_number, d_idx, p_idx, sector_size, false);
    assert_eq!(BigInt::from(sector_size as u64), miner_power(v, &maddr).qa);

    apply_ok(
        v,
        &worker,
        &maddr,
        &TokenAmount::zero(),
        MinerMethod::TerminateSectors as u64,
        Some(TerminateSectorsParams {
            terminations: vec![TerminationDeclaration {
                deadline: d_idx,
                partition: p_idx,
                sectors: make_bitfield(&[sector_number]),
            }],
        }),
    );

    let power = miner_power(v, &maddr);
    assert!(power.raw.is_zero(), "raw power should be fully removed");
    assert!(power.qa.is_zero(), "qa power should be fully removed, leaving no residue");

    assert_invariants(v, &Policy::default(), None);
}

/// A sector carrying `SIMPLE_QA_POWER` but not `FULL_QA_POWER` extends without its power
/// changing, as any other sector does.
#[vm_test]
pub fn simple_qap_sector_extends_without_power_change_test(v: &dyn VM) {
    let addrs = create_accounts(v, 1, &TokenAmount::from_whole(100_000));
    let (worker, owner) = (addrs[0], addrs[0]);
    let seal_proof = RegisteredSealProof::StackedDRG32GiBV1P1;
    let sector_size = seal_proof.sector_size().unwrap();
    let (maddr, _) = create_miner(
        v,
        &owner,
        &worker,
        seal_proof.registered_window_post_proof().unwrap(),
        &TokenAmount::from_whole(10_000),
    );

    v.set_epoch(200);
    let sector_number = 100;
    let (d_idx, p_idx) = create_sector(v, worker, maddr, sector_number, seal_proof);

    // Rewrite into the FIP-0045-era shape: SIMPLE_QA_POWER without FULL_QA_POWER, and fully
    // packed with verified weight so quality derives from that weight instead of the flag. A
    // full sector computes to the same 10x the partition already records, so no power record
    // needs adjusting; and a positive weight is what makes the extension actually restate it.
    let mut si = sector_info(v, &maddr, sector_number);
    si.flags.set(SectorOnChainInfoFlags::FULL_QA_POWER, false);
    si.verified_deal_weight =
        DealWeight::from(sector_size as u64) * (si.expiration - si.power_base_epoch);
    mutate_state(v, &maddr, |st: &mut MinerState| {
        let store = &DynBlockstore::wrap(v.blockstore());
        let mut sectors = Sectors::load(store, &st.sectors).unwrap();
        sectors.store(vec![si.clone()]).unwrap();
        st.sectors = sectors.amt.flush().unwrap();
    });

    let before = sector_info(v, &maddr, sector_number);
    assert!(before.flags.contains(SectorOnChainInfoFlags::SIMPLE_QA_POWER));
    assert!(!before.flags.contains(SectorOnChainInfoFlags::FULL_QA_POWER));
    assert!(!before.verified_deal_weight.is_zero(), "weight must be positive to be restated");
    let before_space = &before.verified_deal_weight / (before.expiration - before.power_base_epoch);
    assert_eq!(BigInt::from(10 * sector_size as u64), miner_power(v, &maddr).qa);
    assert_invariants(v, &Policy::default(), None);

    // The sector was committed at the maximum permitted expiration, so advance before
    // extending or the new expiration exceeds the limit.
    advance_by_deadline_to_epoch_while_proving(
        v,
        &maddr,
        &worker,
        sector_number,
        v.epoch() + 180 * EPOCHS_IN_DAY,
    );
    let new_expiration = before.expiration + 180 * EPOCHS_IN_DAY;
    apply_ok(
        v,
        &worker,
        &maddr,
        &TokenAmount::zero(),
        MinerMethod::ExtendSectorExpiration2 as u64,
        Some(ExtendSectorExpiration2Params {
            extensions: vec![ExpirationExtension2 {
                deadline: d_idx,
                partition: p_idx,
                sectors: make_bitfield(&[sector_number]),
                sectors_with_claims: vec![],
                new_expiration,
            }],
        }),
    );

    let after = sector_info(v, &maddr, sector_number);
    assert_eq!(new_expiration, after.expiration);
    // Weight restated over the new duration, so space and therefore quality are unchanged.
    assert_eq!(
        before_space,
        &after.verified_deal_weight / (after.expiration - after.power_base_epoch)
    );
    assert_eq!(BigInt::from(10 * sector_size as u64), miner_power(v, &maddr).qa);

    assert_invariants(v, &Policy::default(), None);
}

/// A legacy sector's unverified deal spacetime shrinks to the remaining deal term when its
/// expiration is extended, while verified space is restated over the new duration.
#[vm_test]
pub fn legacy_extension_decays_deal_weight_and_restates_verified_weight_test(v: &dyn VM) {
    let addrs = create_accounts(v, 1, &TokenAmount::from_whole(100_000));
    let (worker, owner) = (addrs[0], addrs[0]);
    let seal_proof = RegisteredSealProof::StackedDRG32GiBV1P1;
    let sector_size = seal_proof.sector_size().unwrap();
    let (maddr, _) = create_miner(
        v,
        &owner,
        &worker,
        seal_proof.registered_window_post_proof().unwrap(),
        &TokenAmount::from_whole(10_000),
    );

    v.set_epoch(200);
    let sector_number = 100;
    let (d_idx, p_idx) = create_sector(v, worker, maddr, sector_number, seal_proof);

    let mut weighted = sector_info(v, &maddr, sector_number);
    let old_duration = weighted.expiration - weighted.power_base_epoch;
    let half_sector = DealWeight::from(sector_size as u64 / 2);
    weighted.deal_weight = &half_sector * old_duration;
    weighted.verified_deal_weight = &half_sector * old_duration;
    mutate_state(v, &maddr, |st: &mut MinerState| {
        let store = &DynBlockstore::wrap(v.blockstore());
        let mut sectors = Sectors::load(store, &st.sectors).unwrap();
        sectors.store(vec![weighted.clone()]).unwrap();
        st.sectors = sectors.amt.flush().unwrap();
    });
    make_sector_legacy(v, &maddr, sector_number, d_idx, p_idx, sector_size, false);
    set_sector_daily_fee(v, &maddr, sector_number, d_idx, p_idx, &TokenAmount::from_atto(123));

    let before = sector_info(v, &maddr, sector_number);
    let power_before = miner_power(v, &maddr);
    assert!(before.deal_weight.is_positive());
    assert!(before.verified_deal_weight.is_positive());
    assert_invariants(v, &Policy::default(), None);

    advance_by_deadline_to_epoch_while_proving(
        v,
        &maddr,
        &worker,
        sector_number,
        v.epoch() + 180 * EPOCHS_IN_DAY,
    );
    let extension_epoch = v.epoch();
    let new_expiration = before.expiration + 180 * EPOCHS_IN_DAY;
    let expected_deal_weight =
        &before.deal_weight * (before.expiration - extension_epoch) / old_duration;
    let old_verified_space = &before.verified_deal_weight / old_duration;
    let expected_verified_weight = old_verified_space * (new_expiration - extension_epoch);

    let store = &DynBlockstore::wrap(v.blockstore());
    let state_before: MinerState = get_state(v, &maddr).unwrap();
    let deadline_before =
        state_before.load_deadlines(store).unwrap().load_deadline(store, d_idx).unwrap();

    apply_ok(
        v,
        &worker,
        &maddr,
        &TokenAmount::zero(),
        MinerMethod::ExtendSectorExpiration2 as u64,
        Some(ExtendSectorExpiration2Params {
            extensions: vec![ExpirationExtension2 {
                deadline: d_idx,
                partition: p_idx,
                sectors: make_bitfield(&[sector_number]),
                sectors_with_claims: vec![],
                new_expiration,
            }],
        }),
    );

    let after = sector_info(v, &maddr, sector_number);
    assert_eq!(new_expiration, after.expiration);
    assert_eq!(extension_epoch, after.power_base_epoch);
    assert_eq!(expected_deal_weight, after.deal_weight);
    assert_eq!(expected_verified_weight, after.verified_deal_weight);
    assert_eq!(power_before, miner_power(v, &maddr));

    let state_after: MinerState = get_state(v, &maddr).unwrap();
    let deadline_after =
        state_after.load_deadlines(store).unwrap().load_deadline(store, d_idx).unwrap();
    assert_eq!(deadline_before.live_power, deadline_after.live_power);
    assert_eq!(deadline_before.daily_fee, deadline_after.daily_fee);
    let partition = deadline_after.load_partition(store, p_idx).unwrap();
    let quant = state_after.quant_spec_for_deadline(&Policy::default(), d_idx);
    let queue = ExpirationQueue::new(store, &partition.expirations_epochs, quant).unwrap();
    if let Some(old_set) = queue.amt.get(quant.quantize_up(before.expiration) as u64).unwrap() {
        assert!(!old_set.on_time_sectors.get(sector_number));
    }
    let new_set = queue.amt.get(quant.quantize_up(new_expiration) as u64).unwrap().unwrap();
    assert!(new_set.on_time_sectors.get(sector_number));
    assert_eq!(
        PowerPair { raw: BigInt::from(sector_size as u64), qa: power_before.qa },
        new_set.active_power
    );
    assert_eq!(after.daily_fee, new_set.fee_deduction);

    assert_invariants(v, &Policy::default(), None);
}
