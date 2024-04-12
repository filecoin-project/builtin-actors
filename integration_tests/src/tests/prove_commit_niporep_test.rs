use cid::Cid;

use fvm_ipld_encoding::ipld_block::IpldBlock;
use fvm_ipld_encoding::RawBytes;
use fvm_shared::clock::ChainEpoch;
use fvm_shared::deal::DealID;
use fvm_shared::econ::TokenAmount;
use fvm_shared::piece::PaddedPieceSize;
use fvm_shared::sector::{RegisteredSealProof, SectorNumber};
use num_traits::Zero;

use export_macro::vm_test;
use fil_actor_miner::Method as MinerMethod;
use fil_actor_miner::{PieceChange, ProveCommitSectorsNIParams, SectorNIActivationInfo};
use fil_actor_verifreg::SectorAllocationClaims;
use fil_actors_runtime::cbor::serialize;
// use fil_actors_runtime::runtime::Policy;
use fil_actors_runtime::test_utils::{make_piece_cid, make_sealed_cid};
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
    // let policy = Policy::default();
    let addrs = create_accounts(v, 3, &TokenAmount::from_whole(10_000));
    let seal_proof = RegisteredSealProof::StackedDRG32GiBV1P1_Feat_NiPoRep;
    // let sector_size = seal_proof.sector_size().unwrap();
    let (owner, worker, verifier, client) = (addrs[0], addrs[0], addrs[1], addrs[2]);
    let worker_id = worker.id().unwrap();
    // let client_id = client.id().unwrap();
    let (maddr, _) = create_miner(
        v,
        &owner,
        &worker,
        seal_proof.registered_window_post_proof().unwrap(),
        &TokenAmount::from_whole(8_000),
    );
    // let miner_id = maddr.id().unwrap();

    // Onboard a batch of sectors
    let first_sector_number: SectorNumber = 100;
    let manifests = vec![
        first_sector_number,
        first_sector_number + 1,
        first_sector_number + 2,
        first_sector_number + 3,
        first_sector_number + 4,
    ];
    let cids: Vec<Cid> = manifests
        .iter()
        .map(|sector_num| make_sealed_cid(format!("sn: {}", sector_num).as_bytes()))
        .collect();
    let sectors_info: Vec<SectorNIActivationInfo> = manifests
        .iter()
        .zip(cids)
        .map(|(sector_no, cid)| SectorNIActivationInfo {
            sector_number: *sector_no,
            sealed_cid: cid,
            seal_rand_epoch: 200,
            expiration: 500,
        })
        .collect();

    // Prove-commit NI-PoRep
    let proofs = vec![RawBytes::new(vec![1, 2, 3, 4]); manifests.len()];
    let params = ProveCommitSectorsNIParams {
        sectors: sectors_info,
        seal_proof_type: RegisteredSealProof::StackedDRG32GiBV1P1_Feat_NiPoRep,
        sector_proofs: proofs,
        aggregate_proof: RawBytes::default(),
        aggregate_proof_type: None,
        require_activation_success: true,
    };
    apply_ok(
        v,
        &worker,
        &maddr,
        &TokenAmount::zero(),
        MinerMethod::ProveCommitSectorsNI as u64,
        Some(params.clone()),
    );

    // let events: Vec<EmittedEvent> = manifests
    //     .iter()
    //     .enumerate()
    //     .map(|(i, sa)| {
    //         let unsealed_cid = CompactCommD::empty().get_cid(params.seal_proof_type).unwrap();

    //         let pieces: Vec<(Cid, u64)> = sa.pieces.iter().map(|p| (p.cid, p.size.0)).collect();
    //         Expect::build_sector_activation_event(
    //             "sector-activated",
    //             miner_id,
    //             sa.sector_number,
    //             Some(unsealed_cid),
    //             &pieces,
    //         )
    //     })
    //     .collect();

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
    println!("sectors: {:?}", sectors);
    // for sector in &sectors {
    //     assert_eq!(activation_epoch, sector.activation);
    //     assert_eq!(activation_epoch, sector.power_base_epoch);
    //     assert!(sector.flags.contains(SectorOnChainInfoFlags::SIMPLE_QA_POWER));
    //     assert!(sector.deprecated_deal_ids.is_empty());
    // }
    // let full_sector_weight =
    //     BigInt::from(full_piece_size.0 * (sector_expiry - activation_epoch) as u64);
    // assert_eq!(BigInt::zero(), sectors[0].deal_weight);
    // assert_eq!(BigInt::zero(), sectors[0].verified_deal_weight);
    // assert_eq!(full_sector_weight, sectors[1].deal_weight);
    // assert_eq!(BigInt::zero(), sectors[1].verified_deal_weight);
    // assert_eq!(BigInt::zero(), sectors[2].deal_weight);
    // assert_eq!(full_sector_weight, sectors[2].verified_deal_weight);
    // assert_eq!(full_sector_weight, sectors[3].deal_weight);
    // assert_eq!(BigInt::zero(), sectors[3].verified_deal_weight);
    // assert_eq!(BigInt::zero(), sectors[4].deal_weight);
    // assert_eq!(full_sector_weight / 2, sectors[4].verified_deal_weight);

    // // Brief checks on state consistency between actors.
    // let claims = verifreg_list_claims(v, miner_id);
    // assert_eq!(claims.len(), 3);
    // assert_eq!(first_sector_number + 2, claims[&alloc_ids_s2[0]].sector);
    // assert_eq!(first_sector_number + 2, claims[&alloc_ids_s2[1]].sector);
    // assert_eq!(first_sector_number + 4, claims[&alloc_ids_s4[0]].sector);

    // let deals = market_list_deals(v);
    // assert_eq!(deals.len(), 2);
    // assert_eq!(maddr, deals[&deal_ids_s3[0]].0.provider);
    // assert_eq!(first_sector_number + 3, deals[&deal_ids_s3[0]].1.unwrap().sector_number);
    // assert_eq!(maddr, deals[&deal_ids_s4[0]].0.provider);
    // assert_eq!(first_sector_number + 4, deals[&deal_ids_s4[0]].1.unwrap().sector_number);

    // let sector_deals = market_list_sectors_deals(v, &maddr);
    // assert_eq!(sector_deals.len(), 2);
    // assert_eq!(deal_ids_s3, sector_deals[&(first_sector_number + 3)]);
    // assert_eq!(deal_ids_s4, sector_deals[&(first_sector_number + 4)]);
}

fn no_claims(sector: SectorNumber, expiry: ChainEpoch) -> SectorAllocationClaims {
    SectorAllocationClaims { sector, expiry, claims: vec![] }
}

fn piece_change(cid_seed: &[u8], piece_size: PaddedPieceSize, deal_ids: &[DealID]) -> PieceChange {
    PieceChange {
        data: make_piece_cid(cid_seed),
        size: piece_size,
        payload: serialize(&deal_ids[0], "deal id").unwrap(),
    }
}
