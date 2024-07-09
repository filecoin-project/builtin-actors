use fil_actors_runtime::runtime::Policy;
use fvm_ipld_encoding::ipld_block::IpldBlock;
use fvm_ipld_encoding::RawBytes;
use fvm_shared::bigint::BigInt;
use fvm_shared::econ::TokenAmount;
use fvm_shared::error::ExitCode;
use fvm_shared::sector::{RegisteredAggregateProof, RegisteredSealProof, SectorNumber};
use num_traits::Zero;

use export_macro::vm_test;
use fil_actor_miner::{Method as MinerMethod, SectorOnChainInfoFlags};
use fil_actor_miner::{
    ProveCommitSectorsNIParams, ProveCommitSectorsNIReturn, SectorNIActivationInfo,
};
use fil_actors_runtime::test_utils::make_sealed_cid;
use vm_api::trace::{EmittedEvent, ExpectInvocation};
use vm_api::util::{apply_ok, DynBlockstore};
use vm_api::VM;

use crate::expects::Expect;
use crate::util::{
    advance_by_deadline_to_epoch, advance_by_deadline_to_index, create_accounts, create_miner,
    deadline_state, declare_recovery, override_compute_unsealed_sector_cid, sector_info,
    submit_windowed_post, try_sector_info,
};

#[vm_test]
pub fn prove_commit_ni_whole_success_test(v: &dyn VM) {
    // Expectations depend on the correct unsealed CID for empty sector.
    override_compute_unsealed_sector_cid(v);
    let addrs = create_accounts(v, 3, &TokenAmount::from_whole(10_000));
    let seal_proof = RegisteredSealProof::StackedDRG32GiBV1P2_Feat_NiPoRep;
    let (owner, worker, _, _) = (addrs[0], addrs[0], addrs[1], addrs[2]);
    let worker_id = worker.id().unwrap();
    let (maddr, _) = create_miner(
        v,
        &owner,
        &worker,
        seal_proof.registered_window_post_proof().unwrap(),
        &TokenAmount::from_whole(8_000),
    );
    let miner_id = maddr.id().unwrap();
    let policy = Policy::default();

    // Onboard a batch of sectors
    let seal_rand_epoch = v.epoch();
    let activation_epoch = seal_rand_epoch + policy.max_prove_commit_ni_randomness_lookback / 2;
    let expiration = activation_epoch + policy.min_sector_expiration + 1;
    let first_sector_number: SectorNumber = 100;
    let sector_nos = [
        first_sector_number,
        first_sector_number + 1,
        first_sector_number + 2,
        first_sector_number + 3,
        first_sector_number + 4,
    ];
    let proving_deadline = 7;

    let sectors_info: Vec<SectorNIActivationInfo> = sector_nos
        .iter()
        .map(|sector_number| SectorNIActivationInfo {
            sealing_number: *sector_number,
            sealer_id: miner_id,
            sector_number: *sector_number,
            sealed_cid: make_sealed_cid(format!("sn: {}", sector_number).as_bytes()),
            seal_rand_epoch,
            expiration,
        })
        .collect();

    // Prove-commit NI-PoRep
    let aggregate_proof = RawBytes::new(vec![1, 2, 3, 4]);
    let params = ProveCommitSectorsNIParams {
        sectors: sectors_info.clone(),
        seal_proof_type: RegisteredSealProof::StackedDRG32GiBV1P2_Feat_NiPoRep,
        aggregate_proof,
        aggregate_proof_type: RegisteredAggregateProof::SnarkPackV2,
        proving_deadline,
        require_activation_success: true,
    };

    v.set_epoch(activation_epoch);

    let pcsni_ret: ProveCommitSectorsNIReturn = apply_ok(
        v,
        &worker,
        &maddr,
        &TokenAmount::zero(),
        MinerMethod::ProveCommitSectorsNI as u64,
        Some(params.clone()),
    )
    .deserialize()
    .unwrap();

    assert_eq!(pcsni_ret.activation_results.size(), 5);
    assert!(pcsni_ret.activation_results.all_ok());
    assert_eq!(pcsni_ret.activation_results.codes(), [ExitCode::OK].repeat(5));

    let events: Vec<EmittedEvent> = sector_nos
        .iter()
        .map(|sector_number| {
            Expect::build_sector_activation_event(
                "sector-activated",
                miner_id,
                *sector_number,
                None,
                &vec![],
            )
        })
        .collect();

    ExpectInvocation {
        from: worker_id,
        to: maddr,
        method: MinerMethod::ProveCommitSectorsNI as u64,
        params: Some(IpldBlock::serialize_cbor(&params).unwrap()),
        subinvocs: None,
        events: Some(events),
        ..Default::default()
    }
    .matches(v.take_invocations().last().unwrap());

    // Check sector state.
    let sectors = sector_nos
        .iter()
        .map(|sector_number| sector_info(v, &maddr, *sector_number))
        .collect::<Vec<_>>();

    for (on_chain_sector, input_sector) in sectors.iter().zip(sectors_info) {
        assert_eq!(input_sector.sector_number, on_chain_sector.sector_number);
        assert_eq!(params.seal_proof_type, on_chain_sector.seal_proof);
        assert_eq!(input_sector.sealed_cid, on_chain_sector.sealed_cid);
        assert!(on_chain_sector.deprecated_deal_ids.is_empty());
        assert_eq!(activation_epoch, on_chain_sector.activation);
        assert_eq!(input_sector.expiration, on_chain_sector.expiration);
        assert_eq!(BigInt::zero(), on_chain_sector.deal_weight);
        assert_eq!(BigInt::zero(), on_chain_sector.verified_deal_weight);
        assert_eq!(activation_epoch, on_chain_sector.power_base_epoch);
        assert!(on_chain_sector.flags.contains(SectorOnChainInfoFlags::SIMPLE_QA_POWER));
    }

    let deadline = deadline_state(v, &maddr, proving_deadline);
    assert_eq!(deadline.live_sectors, sector_nos.len() as u64);
}

#[vm_test]
pub fn prove_commit_ni_partial_success_not_required_test(v: &dyn VM) {
    // Expectations depend on the correct unsealed CID for empty sector.
    override_compute_unsealed_sector_cid(v);
    let addrs = create_accounts(v, 3, &TokenAmount::from_whole(10_000));
    let seal_proof = RegisteredSealProof::StackedDRG32GiBV1P2_Feat_NiPoRep;
    let (owner, worker, _, _) = (addrs[0], addrs[0], addrs[1], addrs[2]);
    let worker_id = worker.id().unwrap();
    let (maddr, _) = create_miner(
        v,
        &owner,
        &worker,
        seal_proof.registered_window_post_proof().unwrap(),
        &TokenAmount::from_whole(8_000),
    );
    let miner_id = maddr.id().unwrap();
    let policy = Policy::default();

    // Onboard a batch of sectors
    let seal_rand_epoch = v.epoch();
    let activation_epoch = seal_rand_epoch + policy.max_prove_commit_ni_randomness_lookback / 2;
    let expiration = activation_epoch + policy.min_sector_expiration + 1;
    let first_sector_number: SectorNumber = 100;
    let sector_nos: Vec<_> = (first_sector_number..first_sector_number + 20).collect();
    let proving_deadline = 7;

    let mut sectors_info: Vec<SectorNIActivationInfo> = sector_nos
        .iter()
        .map(|sector_number| SectorNIActivationInfo {
            sealing_number: *sector_number,
            sealer_id: miner_id,
            sector_number: *sector_number,
            sealed_cid: make_sealed_cid(format!("sn: {}", sector_number).as_bytes()),
            seal_rand_epoch,
            expiration,
        })
        .collect();

    // non-fatal errors
    sectors_info[0].seal_rand_epoch =
        v.epoch() - policy.max_prove_commit_ni_randomness_lookback - 1;
    sectors_info[2].sealer_id = miner_id + 1;

    let invalid_sector_nos = vec![sector_nos[0], sector_nos[2]];
    let valid_sector_nos: Vec<_> =
        sector_nos.iter().enumerate().filter(|&(i, _)| i != 0 && i != 2).map(|(_, &v)| v).collect();
    let valid_sectors_info: Vec<_> = sectors_info
        .iter()
        .enumerate()
        .filter(|&(i, _)| i != 0 && i != 2)
        .map(|(_, v)| v.clone())
        .collect();

    // Prove-commit NI-PoRep
    let aggregate_proof = RawBytes::new(vec![1, 2, 3, 4]);
    let params = ProveCommitSectorsNIParams {
        sectors: sectors_info.clone(),
        seal_proof_type: RegisteredSealProof::StackedDRG32GiBV1P2_Feat_NiPoRep,
        aggregate_proof,
        aggregate_proof_type: RegisteredAggregateProof::SnarkPackV2,
        proving_deadline,
        require_activation_success: false,
    };

    v.set_epoch(activation_epoch);

    let pcsni_ret: ProveCommitSectorsNIReturn = apply_ok(
        v,
        &worker,
        &maddr,
        &TokenAmount::zero(),
        MinerMethod::ProveCommitSectorsNI as u64,
        Some(params.clone()),
    )
    .deserialize()
    .unwrap();

    assert_eq!(pcsni_ret.activation_results.size(), sector_nos.len());
    assert!(!pcsni_ret.activation_results.all_ok());
    assert_eq!(
        pcsni_ret.activation_results.codes(),
        (0..sector_nos.len())
            .map(|i| if i == 0 || i == 2 { ExitCode::USR_ILLEGAL_ARGUMENT } else { ExitCode::OK })
            .collect::<Vec<_>>()
    );

    let events: Vec<EmittedEvent> = valid_sector_nos
        .iter()
        .map(|sector_number| {
            Expect::build_sector_activation_event(
                "sector-activated",
                miner_id,
                *sector_number,
                None,
                &vec![],
            )
        })
        .collect();

    ExpectInvocation {
        from: worker_id,
        to: maddr,
        method: MinerMethod::ProveCommitSectorsNI as u64,
        params: Some(IpldBlock::serialize_cbor(&params).unwrap()),
        subinvocs: None,
        events: Some(events),
        ..Default::default()
    }
    .matches(v.take_invocations().last().unwrap());

    // Check sector state.
    invalid_sector_nos
        .iter()
        .for_each(|sector_number| assert!(try_sector_info(v, &maddr, *sector_number).is_none()));

    let sectors = valid_sector_nos
        .iter()
        .map(|sector_number| sector_info(v, &maddr, *sector_number))
        .collect::<Vec<_>>();

    for (on_chain_sector, input_sector) in sectors.iter().zip(valid_sectors_info) {
        assert_eq!(input_sector.sector_number, on_chain_sector.sector_number);
        assert_eq!(params.seal_proof_type, on_chain_sector.seal_proof);
        assert_eq!(input_sector.sealed_cid, on_chain_sector.sealed_cid);
        assert!(on_chain_sector.deprecated_deal_ids.is_empty());
        assert_eq!(activation_epoch, on_chain_sector.activation);
        assert_eq!(input_sector.expiration, on_chain_sector.expiration);
        assert_eq!(BigInt::zero(), on_chain_sector.deal_weight);
        assert_eq!(BigInt::zero(), on_chain_sector.verified_deal_weight);
        assert_eq!(activation_epoch, on_chain_sector.power_base_epoch);
        assert!(on_chain_sector.flags.contains(SectorOnChainInfoFlags::SIMPLE_QA_POWER));
    }

    // Check if sectors are properly assigned to deadline
    let deadline = deadline_state(v, &maddr, proving_deadline);
    assert_eq!(deadline.live_sectors, valid_sector_nos.len() as u64);

    let store = &DynBlockstore::wrap(v.blockstore());
    let partition = deadline.load_partition(store, 0).unwrap();
    for sector_number in invalid_sector_nos {
        assert!(!partition.sectors.get(sector_number));
    }
    for sector_number in &valid_sector_nos {
        assert!(partition.sectors.get(*sector_number));
        assert!(partition.unproven.get(*sector_number));
    }

    // Advance to proving deadline and submit WindowPoSt
    let deadline_info = advance_by_deadline_to_index(v, &maddr, proving_deadline);

    let deadline = deadline_state(v, &maddr, proving_deadline);
    let submissions = deadline.optimistic_proofs_amt(store).unwrap();
    assert_eq!(submissions.count(), 0);

    submit_windowed_post(v, &worker, &maddr, deadline_info, 0, Some(partition.unproven_power));

    // Check if post is registered in deadline's optimistic submissions
    let deadline = deadline_state(v, &maddr, proving_deadline);
    let submissions = deadline.optimistic_proofs_amt(store).unwrap();
    assert_eq!(submissions.count(), 1);

    // Move to next deadline and check if sectors are active
    let deadline_info = advance_by_deadline_to_index(
        v,
        &maddr,
        proving_deadline + 1 % policy.wpost_proving_period as u64,
    );
    let deadline = deadline_state(v, &maddr, proving_deadline);
    let partition = deadline.load_partition(store, 0).unwrap();

    for sector_number in &valid_sector_nos {
        assert!(partition.active_sectors().get(*sector_number));
        assert!(partition.faults.is_empty());
    }

    // Move to next deadline period while skipping window post submission
    // and check if sectors are faulty
    advance_by_deadline_to_epoch(v, &maddr, deadline_info.close + policy.wpost_proving_period);
    let deadline = deadline_state(v, &maddr, proving_deadline);
    let partition = deadline.load_partition(store, 0).unwrap();

    for sector_number in &valid_sector_nos {
        assert!(partition.faults.get(*sector_number));
        assert!(partition.active_sectors().is_empty());
        assert!(partition.recoveries.is_empty());
    }

    // Recover faulty sectors
    for sector_number in &valid_sector_nos {
        declare_recovery(v, &worker, &maddr, proving_deadline, 0, *sector_number);
    }
    let deadline = deadline_state(v, &maddr, proving_deadline);
    let partition = deadline.load_partition(store, 0).unwrap();
    for sector_number in &valid_sector_nos {
        assert!(partition.faults.get(*sector_number));
        assert!(partition.recoveries.get(*sector_number));
    }

    // Move to next deadline period and prove sectors
    let deadline_info = advance_by_deadline_to_index(v, &maddr, proving_deadline);
    submit_windowed_post(v, &worker, &maddr, deadline_info, 0, None);

    // Move to next deadline and check if sectors are active
    advance_by_deadline_to_index(
        v,
        &maddr,
        proving_deadline + 1 % policy.wpost_proving_period as u64,
    );
    let deadline = deadline_state(v, &maddr, proving_deadline);
    let partition = deadline.load_partition(store, 0).unwrap();

    for sector_number in &valid_sector_nos {
        assert!(partition.active_sectors().get(*sector_number));
        assert!(partition.faults.is_empty());
        assert!(partition.recoveries.is_empty());
    }
}

#[vm_test]
pub fn prove_commit_ni_next_deadline_post_required_test(v: &dyn VM) {
    // Expectations depend on the correct unsealed CID for empty sector.
    override_compute_unsealed_sector_cid(v);
    let addrs = create_accounts(v, 3, &TokenAmount::from_whole(10_000));
    let seal_proof = RegisteredSealProof::StackedDRG32GiBV1P2_Feat_NiPoRep;
    let (owner, worker, _, _) = (addrs[0], addrs[0], addrs[1], addrs[2]);
    let worker_id = worker.id().unwrap();
    let (maddr, _) = create_miner(
        v,
        &owner,
        &worker,
        seal_proof.registered_window_post_proof().unwrap(),
        &TokenAmount::from_whole(8_000),
    );
    let miner_id = maddr.id().unwrap();
    let policy = Policy::default();

    // Onboard a single sector
    let seal_rand_epoch = v.epoch();
    let activation_epoch = seal_rand_epoch + policy.max_prove_commit_ni_randomness_lookback / 2;
    let expiration = activation_epoch + policy.min_sector_expiration + 1;
    let sector_number: SectorNumber = 101;
    let proving_deadline = 14;

    let ni_sector_info = SectorNIActivationInfo {
        sealing_number: sector_number,
        sealer_id: miner_id,
        sector_number,
        sealed_cid: make_sealed_cid(format!("sn: {}", sector_number).as_bytes()),
        seal_rand_epoch,
        expiration,
    };

    // Prove-commit NI-PoRep
    let aggregate_proof = RawBytes::new(vec![1, 2, 3, 4]);
    let params = ProveCommitSectorsNIParams {
        sectors: vec![ni_sector_info.clone()],
        seal_proof_type: RegisteredSealProof::StackedDRG32GiBV1P2_Feat_NiPoRep,
        aggregate_proof,
        aggregate_proof_type: RegisteredAggregateProof::SnarkPackV2,
        proving_deadline,
        require_activation_success: true,
    };

    v.set_epoch(activation_epoch);

    let pcsni_ret: ProveCommitSectorsNIReturn = apply_ok(
        v,
        &worker,
        &maddr,
        &TokenAmount::zero(),
        MinerMethod::ProveCommitSectorsNI as u64,
        Some(params.clone()),
    )
    .deserialize()
    .unwrap();

    assert_eq!(pcsni_ret.activation_results.size(), 1);
    assert!(pcsni_ret.activation_results.all_ok());
    assert_eq!(pcsni_ret.activation_results.codes(), [ExitCode::OK]);

    ExpectInvocation {
        from: worker_id,
        to: maddr,
        method: MinerMethod::ProveCommitSectorsNI as u64,
        params: Some(IpldBlock::serialize_cbor(&params).unwrap()),
        subinvocs: None,
        events: Some(vec![Expect::build_sector_activation_event(
            "sector-activated",
            miner_id,
            sector_number,
            None,
            &vec![],
        )]),
        ..Default::default()
    }
    .matches(v.take_invocations().last().unwrap());

    // Check sector state.
    let on_chain_sector = sector_info(v, &maddr, sector_number);
    assert_eq!(ni_sector_info.sector_number, on_chain_sector.sector_number);
    assert_eq!(params.seal_proof_type, on_chain_sector.seal_proof);
    assert_eq!(ni_sector_info.sealed_cid, on_chain_sector.sealed_cid);
    assert!(on_chain_sector.deprecated_deal_ids.is_empty());
    assert_eq!(activation_epoch, on_chain_sector.activation);
    assert_eq!(ni_sector_info.expiration, on_chain_sector.expiration);
    assert_eq!(BigInt::zero(), on_chain_sector.deal_weight);
    assert_eq!(BigInt::zero(), on_chain_sector.verified_deal_weight);
    assert_eq!(activation_epoch, on_chain_sector.power_base_epoch);
    assert!(on_chain_sector.flags.contains(SectorOnChainInfoFlags::SIMPLE_QA_POWER));

    // Check if sector is properly assigned to the deadline
    let deadline = deadline_state(v, &maddr, proving_deadline);
    assert_eq!(deadline.live_sectors, 1u64);

    let store = &DynBlockstore::wrap(v.blockstore());
    let partition = deadline.load_partition(store, 0).unwrap();
    assert!(partition.sectors.get(sector_number));
    assert!(partition.unproven.get(sector_number));

    // Move past the first proving deadline while skipping window post submission
    // and check if the sector is faulty
    let deadline_info = advance_by_deadline_to_index(v, &maddr, proving_deadline);
    advance_by_deadline_to_epoch(v, &maddr, deadline_info.close + policy.wpost_proving_period);
    let deadline = deadline_state(v, &maddr, proving_deadline);
    let partition = deadline.load_partition(store, 0).unwrap();

    assert!(partition.faults.get(sector_number));
    assert!(partition.active_sectors().is_empty());
    assert!(partition.recoveries.is_empty());

    // Recover faulty sectors
    declare_recovery(v, &worker, &maddr, proving_deadline, 0, sector_number);
    let deadline = deadline_state(v, &maddr, proving_deadline);
    let partition = deadline.load_partition(store, 0).unwrap();
    assert!(partition.faults.get(sector_number));
    assert!(partition.recoveries.get(sector_number));

    // Move to the next proving deadline and submit WindowPoSt
    let deadline_info = advance_by_deadline_to_index(v, &maddr, proving_deadline);
    submit_windowed_post(v, &worker, &maddr, deadline_info, 0, None);

    // Move to next deadline and check if sectors are active
    advance_by_deadline_to_index(
        v,
        &maddr,
        proving_deadline + 1 % policy.wpost_proving_period as u64,
    );
    let deadline = deadline_state(v, &maddr, proving_deadline);
    let partition = deadline.load_partition(store, 0).unwrap();

    assert!(partition.active_sectors().get(sector_number));
    assert!(partition.faults.is_empty());
    assert!(partition.recoveries.is_empty());

    // Move to the next proving deadline and submit WindowPoSt
    let deadline_info = advance_by_deadline_to_index(v, &maddr, proving_deadline);
    let deadline = deadline_state(v, &maddr, proving_deadline);
    let submissions = deadline.optimistic_proofs_amt(store).unwrap();
    assert_eq!(submissions.count(), 0);

    submit_windowed_post(v, &worker, &maddr, deadline_info, 0, Some(partition.unproven_power));

    // Check if post is registered in deadline's optimistic submissions
    let deadline = deadline_state(v, &maddr, proving_deadline);
    let submissions = deadline.optimistic_proofs_amt(store).unwrap();
    assert_eq!(submissions.count(), 1);
}
