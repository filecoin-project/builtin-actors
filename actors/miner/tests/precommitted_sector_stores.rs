use cid::Cid;
use fil_actor_miner::{SectorPreCommitInfo, SectorPreCommitOnChainInfo};
use fil_actors_runtime::test_utils::make_sealed_cid;
use fvm_shared::clock::ChainEpoch;
use fvm_shared::econ::TokenAmount;
use fvm_shared::sector::RegisteredSealProof;
use fvm_shared::sector::SectorNumber;

mod state_harness;
use state_harness::*;

#[test]
fn put_get_and_delete() {
    let mut h = StateHarness::new(0);

    let pc1 =
        new_pre_commit_on_chain(1, make_sealed_cid("1".as_bytes()), TokenAmount::from_atto(1), 1);
    h.put_precommitted_sectors(vec![pc1.clone()]).unwrap();
    assert_eq!(pc1, h.get_precommit(1));

    let pc2 =
        new_pre_commit_on_chain(2, make_sealed_cid("2".as_bytes()), TokenAmount::from_atto(1), 1);
    h.put_precommitted_sectors(vec![pc2.clone()]).unwrap();
    assert_eq!(pc2, h.get_precommit(2));

    let pc3 =
        new_pre_commit_on_chain(3, make_sealed_cid("2".as_bytes()), TokenAmount::from_atto(1), 1);
    let pc4 =
        new_pre_commit_on_chain(4, make_sealed_cid("2".as_bytes()), TokenAmount::from_atto(1), 1);
    h.put_precommitted_sectors(vec![pc3.clone(), pc4.clone()]).unwrap();
    assert_eq!(pc3, h.get_precommit(3));
    assert_eq!(pc4, h.get_precommit(4));

    h.delete_precommitted_sectors(&[1]).unwrap();
    assert!(!h.has_precommit(1));
    assert!(h.has_precommit(2));
}

#[test]
fn delete_nonexistent_value_returns_an_error() {
    let mut h = StateHarness::new(0);
    assert!(h.delete_precommitted_sectors(&[1]).is_err());
}

#[test]
fn has_nonexistent_value_returns_false() {
    let h = StateHarness::new(0);
    assert!(!h.has_precommit(1));
}

#[test]
fn duplicate_put_rejected() {
    let mut h = StateHarness::new(0);

    let pc1 =
        new_pre_commit_on_chain(1, make_sealed_cid("1".as_bytes()), TokenAmount::from_atto(1), 1);

    // In sequence
    assert!(h.put_precommitted_sectors(vec![pc1.clone()]).is_ok());
    assert!(h.put_precommitted_sectors(vec![pc1]).is_err());

    // In batch
    let pc2 =
        new_pre_commit_on_chain(2, make_sealed_cid("2".as_bytes()), TokenAmount::from_atto(1), 1);
    assert!(h.put_precommitted_sectors(vec![pc2.clone(), pc2]).is_err());
}

fn new_pre_commit_on_chain(
    sector_number: SectorNumber,
    sealed_cid: Cid,
    deposit: TokenAmount,
    epoch: ChainEpoch,
) -> SectorPreCommitOnChainInfo {
    let info = SectorPreCommitInfo {
        seal_proof: RegisteredSealProof::StackedDRG32GiBV1P1,
        sector_number,
        sealed_cid,
        seal_rand_epoch: 1,
        expiration: 1,
        ..SectorPreCommitInfo::default()
    };
    SectorPreCommitOnChainInfo { info, pre_commit_deposit: deposit, pre_commit_epoch: epoch }
}
