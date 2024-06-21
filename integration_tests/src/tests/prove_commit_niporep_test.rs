use fil_actors_runtime::runtime::Policy;
use fvm_ipld_encoding::ipld_block::IpldBlock;
use fvm_ipld_encoding::RawBytes;
use fvm_shared::bigint::BigInt;
use fvm_shared::econ::TokenAmount;
use fvm_shared::sector::{RegisteredAggregateProof, RegisteredSealProof, SectorNumber};
use num_traits::Zero;

use export_macro::vm_test;
use fil_actor_miner::{Method as MinerMethod, SectorOnChainInfoFlags};
use fil_actor_miner::{ProveCommitSectorsNIParams, SectorNIActivationInfo};
use fil_actors_runtime::test_utils::make_sealed_cid;
use vm_api::trace::{EmittedEvent, ExpectInvocation};
use vm_api::util::apply_ok;
use vm_api::VM;

use crate::expects::Expect;
use crate::util::{
    create_accounts, create_miner, deadline_state, override_compute_unsealed_sector_cid,
    sector_info, try_sector_info,
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

    apply_ok(
        v,
        &worker,
        &maddr,
        &TokenAmount::zero(),
        MinerMethod::ProveCommitSectorsNI as u64,
        Some(params.clone()),
    );

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

    // Checks on sector state.
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
    let sector_nos = [
        first_sector_number,
        first_sector_number + 1,
        first_sector_number + 2,
        first_sector_number + 3,
        first_sector_number + 4,
    ];
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

    sectors_info[0].seal_rand_epoch = activation_epoch;

    let invalid_sector_nos = sector_nos.iter().take(1).copied().collect::<Vec<_>>();
    let valid_sector_nos = sector_nos.iter().skip(1).copied().collect::<Vec<_>>();
    let valid_sectors_info = sectors_info.iter().skip(1).cloned().collect::<Vec<_>>();

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

    apply_ok(
        v,
        &worker,
        &maddr,
        &TokenAmount::zero(),
        MinerMethod::ProveCommitSectorsNI as u64,
        Some(params.clone()),
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

    // Checks on sector state.
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

    let deadline = deadline_state(v, &maddr, proving_deadline);
    assert_eq!(deadline.live_sectors, valid_sector_nos.len() as u64);
}
