use cid::Cid;
use fil_actor_miner::SectorOnChainInfo;
use fil_actors_runtime::test_utils::*;
use fvm_ipld_bitfield::BitField;
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

#[test]
fn put_get_and_delete() {
    let mut h = StateHarness::new(0);

    let sector_no = 1u64;
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

#[test]
fn delete_nonexistent_value_returns_an_error() {
    let mut h = StateHarness::new(ChainEpoch::from(0));

    let sector_no = 1u64;
    let mut bf = BitField::new();
    bf.set(sector_no as u64);

    assert!(h.st.delete_sectors(&h.store, &bf).is_err());
}

#[test]
fn get_nonexistent_value_returns_false() {
    let h = StateHarness::new(ChainEpoch::from(0));

    let sector_number = 1u64;
    assert!(!h.has_sector_number(sector_number as u64));
}

#[test]
fn iterate_and_delete_multiple_sectors() {
    let mut h = StateHarness::new(ChainEpoch::from(0));

    // set of sectors, the larger numbers here are not significant
    let sector_nos = vec![100, 200, 300, 400, 500, 600, 700, 800, 900, 1000];

    // put all the sectors in the store
    for (i, s) in sector_nos.iter().enumerate() {
        h.put_sector(&new_sector_on_chain_info(
            *s as u64,
            make_sealed_cid(format!("{}", i).as_bytes()),
            BigInt::from(i),
            i as i64,
        ))
    }

    let mut sector_no_idx = 0;
    h.st.for_each_sector(&h.store, |si| {
        assert_eq!(sector_nos[sector_no_idx], si.sector_number);
        sector_no_idx += 1;
        Ok(())
    })
    .unwrap();

    // ensure we iterated over the expected number of sectors
    assert_eq!(sector_nos.len(), sector_no_idx);
    h.delete_sectors(sector_nos.clone());
    for s in sector_nos {
        assert!(!h.has_sector_number(s));
    }
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
        deal_weight: weight.clone(),
        verified_deal_weight: weight,
        initial_pledge: TokenAmount::from(0),
        expected_day_reward: TokenAmount::from(0),
        expected_storage_pledge: TokenAmount::from(0),
        replaced_sector_age: ChainEpoch::from(0),
        replaced_day_reward: BigInt::zero(),
        sector_key_cid: None,
    }
}
