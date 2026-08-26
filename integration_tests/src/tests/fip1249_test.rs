use export_macro::vm_test;
use fil_actor_miner::{
    Method as MinerMethod, ProveCommitSectorsNIParams, ProveCommitSectorsNIReturn,
    SectorNIActivationInfo, SectorOnChainInfoFlags, max_prove_commit_duration,
};
use fil_actor_multisig::Method as MultisigMethod;
use fil_actor_verifreg::{AddVerifiedClientParams, Method as VerifregMethod, VerifierParams};
use fil_actors_runtime::cbor::serialize;
use fil_actors_runtime::runtime::Policy;
use fil_actors_runtime::test_utils::make_sealed_cid;
use fil_actors_runtime::{EPOCHS_IN_DAY, VERIFIED_REGISTRY_ACTOR_ADDR};
use fvm_ipld_encoding::RawBytes;
use fvm_shared::bigint::BigInt;
use fvm_shared::econ::TokenAmount;
use fvm_shared::error::ExitCode;
use fvm_shared::piece::PaddedPieceSize;
use fvm_shared::sector::{
    RegisteredAggregateProof, RegisteredSealProof, SectorNumber, StoragePower,
};
use num_traits::Zero;

use fil_actor_multisig::ProposeParams;
use fil_actor_verifreg::State as VerifregState;
use vm_api::VM;
use vm_api::util::{DynBlockstore, apply_code, apply_ok, get_state};

use crate::util::{
    PrecommitMetadata, advance_by_deadline_to_epoch, advance_by_deadline_to_index,
    advance_to_proving_deadline, assert_invariants, create_accounts, create_miner, cron_tick,
    datacap_get_balance, make_piece_manifests_from_deal_ids, market_add_balance,
    market_publish_deal, miner_power, miner_precommit_one_sector_v2, miner_prove_sector,
    override_compute_unsealed_sector_cid, precommit_meta_data_from_deals, sector_info,
    submit_windowed_post,
};

/// FIP-1249: A new CC sector committed via ProveCommitSectors3 gets 10x QA power.
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

/// FIP-1249: A sector committed via ProveCommitSectorsNI gets 10x QA power.
#[vm_test]
pub fn ni_sector_gets_10x_test(v: &dyn VM) {
    override_compute_unsealed_sector_cid(v);
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

/// FIP-1249: AddVerifier and AddVerifiedClient on verifreg actor are deprecated.
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

/// FIP-1249: Publishing a verified deal does NOT transfer datacap tokens.
/// The sector still gets 10x QA power (same as CC sectors).
#[vm_test]
pub fn verified_deal_no_datacap_ops_test(v: &dyn VM) {
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
        "fip1249-verified-deal".to_string(),
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

    // Advance to proving deadline and submit PoSt
    let (deadline_info, partition_index) = advance_to_proving_deadline(v, &maddr, sector_number);

    // FIP-1249: All new sectors get 10x QA power regardless of deal content
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
        "QA power should be 10x raw power (same as CC with FIP-1249)"
    );

    assert_invariants(v, &Policy::default(), None);
}
