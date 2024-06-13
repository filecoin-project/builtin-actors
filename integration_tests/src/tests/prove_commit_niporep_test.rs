use fil_actors_runtime::EPOCHS_IN_YEAR;
use fvm_ipld_encoding::ipld_block::IpldBlock;
use fvm_ipld_encoding::RawBytes;
use fvm_shared::bigint::BigInt;
use fvm_shared::econ::TokenAmount;
use fvm_shared::sector::{RegisteredAggregateProof, RegisteredSealProof, SectorNumber};
use log::info;
use num_traits::Zero;

use export_macro::vm_test;
use fil_actor_miner::{Method as MinerMethod, SectorOnChainInfoFlags};
use fil_actor_miner::{ProveCommitSectorsNIParams, SectorNIActivationInfo};
use fil_actors_runtime::test_utils::make_sealed_cid;
use vm_api::trace::ExpectInvocation;
use vm_api::util::apply_ok;
use vm_api::VM;

use crate::util::{
    create_accounts, create_miner, override_compute_unsealed_sector_cid, sector_info,
};

#[vm_test]
pub fn prove_commit_sectors_niporep_test(v: &dyn VM) {
    // Expectations depend on the correct unsealed CID for empty sector.
    override_compute_unsealed_sector_cid(v);
    let addrs = create_accounts(v, 3, &TokenAmount::from_whole(10_000));
    let seal_proof = RegisteredSealProof::StackedDRG32GiBV1P1_Feat_NiPoRep;
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

    // Onboard a batch of sectors
    let expiration = v.epoch() + 2 * EPOCHS_IN_YEAR;
    let seal_rand_epoch = v.epoch() - 1;
    let first_sector_number: SectorNumber = 100;
    let manifests = vec![
        first_sector_number,
        first_sector_number + 1,
        first_sector_number + 2,
        first_sector_number + 3,
        first_sector_number + 4,
    ];

    let sectors_info: Vec<SectorNIActivationInfo> = manifests
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
    let proofs = vec![RawBytes::new(vec![1, 2, 3, 4]); manifests.len()];
    let params = ProveCommitSectorsNIParams {
        sectors: sectors_info.clone(),
        seal_proof_type: RegisteredSealProof::StackedDRG32GiBV1P1_Feat_NiPoRep,
        sector_proofs: proofs,
        aggregate_proof: RawBytes::default(),
        aggregate_proof_type: None,
        require_activation_success: true,
    };

    let activation_epoch = v.epoch() + EPOCHS_IN_YEAR / 2;
    v.set_epoch(activation_epoch);

    apply_ok(
        v,
        &worker,
        &maddr,
        &TokenAmount::zero(),
        MinerMethod::ProveCommitSectorsNI as u64,
        Some(params.clone()),
    );

    ExpectInvocation {
        from: worker_id,
        to: maddr,
        method: MinerMethod::ProveCommitSectorsNI as u64,
        params: Some(IpldBlock::serialize_cbor(&params).unwrap()),
        subinvocs: None,
        events: Vec::new(),
        ..Default::default()
    }
    .matches(v.take_invocations().last().unwrap());

    // Checks on sector state.
    let sectors = manifests
        .iter()
        .map(|sector_number| sector_info(v, &maddr, *sector_number))
        .collect::<Vec<_>>();
    info!("sectors: {:#?}", sectors);
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
}

#[vm_test]
pub fn prove_commit_sectors_aggregate_niporep_test(v: &dyn VM) {
    // Expectations depend on the correct unsealed CID for empty sector.
    override_compute_unsealed_sector_cid(v);
    let addrs = create_accounts(v, 3, &TokenAmount::from_whole(10_000));
    let seal_proof = RegisteredSealProof::StackedDRG32GiBV1P1_Feat_NiPoRep;
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

    // Onboard a batch of sectors
    let expiration = v.epoch() + 2 * EPOCHS_IN_YEAR;
    let seal_rand_epoch = v.epoch() - 1;
    let first_sector_number: SectorNumber = 100;
    let manifests = vec![
        first_sector_number,
        first_sector_number + 1,
        first_sector_number + 2,
        first_sector_number + 3,
        first_sector_number + 4,
    ];

    let sectors_info: Vec<SectorNIActivationInfo> = manifests
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
        seal_proof_type: RegisteredSealProof::StackedDRG32GiBV1P1_Feat_NiPoRep,
        sector_proofs: Vec::new(),
        aggregate_proof,
        aggregate_proof_type: Some(RegisteredAggregateProof::SnarkPackV2),
        require_activation_success: true,
    };

    let activation_epoch = v.epoch() + EPOCHS_IN_YEAR / 2;
    v.set_epoch(activation_epoch);

    apply_ok(
        v,
        &worker,
        &maddr,
        &TokenAmount::zero(),
        MinerMethod::ProveCommitSectorsNI as u64,
        Some(params.clone()),
    );

    ExpectInvocation {
        from: worker_id,
        to: maddr,
        method: MinerMethod::ProveCommitSectorsNI as u64,
        params: Some(IpldBlock::serialize_cbor(&params).unwrap()),
        subinvocs: None,
        events: Vec::new(),
        ..Default::default()
    }
    .matches(v.take_invocations().last().unwrap());

    // Checks on sector state.
    let sectors = manifests
        .iter()
        .map(|sector_number| sector_info(v, &maddr, *sector_number))
        .collect::<Vec<_>>();
    info!("sectors: {:#?}", sectors);
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
}
