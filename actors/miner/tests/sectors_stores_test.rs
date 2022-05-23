use cid::Cid;
use fil_actor_miner::SectorOnChainInfo;
use fil_actors_runtime::test_utils::*;
use fvm_shared::{
    bigint::BigInt,
    clock::ChainEpoch,
    econ::TokenAmount,
    sector::{RegisteredSealProof, SectorNumber},
};

use num_traits::Zero;

mod state_harness;
mod util;
use state_harness::*;
use util::*;

#[test]
fn put_get_and_delete() {
    let mut h = StateHarness::new(0);

    let sector_no = SectorNumber::from(1u64);
    let sector_info_1 = new_sector_on_chain_info(
        sector_no,
        make_sealed_cid(b"1"),
        BigInt::from(1u64),
        ChainEpoch::from(1),
    );
    let sector_info_2 = new_sector_on_chain_info(
        sector_no,
        make_sealed_cid(b"2"),
        BigInt::from(2u64),
        ChainEpoch::from(2),
    );

    h.put_sector(&sector_info_1);
    assert!(h.has_sector_number(sector_no));
    let out = h.get_sector(sector_no);
    assert_eq!(sector_info_1, out);

    h.put_sector(&sector_info_2);
    assert!(h.has_sector_number(sector_no));
    let out = h.get_sector(sector_no);
    assert_eq!(sector_info_2, out);

    h.delete_sectors(vec![sector_no as u64]);
    assert!(!h.has_sector_number(sector_no));
}

// returns a unique SectorOnChainInfo with each invocation with SectorNumber set to `sectorNo`.
fn new_sector_on_chain_info(
    sector_no: SectorNumber,
    sealed_cid: Cid,
    weight: BigInt,
    activation: ChainEpoch,
) -> SectorOnChainInfo {
    SectorOnChainInfo {
        sector_number: sector_no,
        seal_proof: RegisteredSealProof::StackedDRG32GiBV1,
        sealed_cid,
        deal_ids: vec![],
        activation,
        expiration: ChainEpoch::from(1),
        deal_weight: weight,
        verified_deal_weight: weight,
        initial_pledge: TokenAmount::from(0),
        expected_day_reward: TokenAmount::from(0),
        expected_storage_pledge: TokenAmount::from(0),
        replaced_sector_age: ChainEpoch::from(0),
        replaced_day_reward: BigInt::zero(),
        sector_key_cid: None,
    }
}
