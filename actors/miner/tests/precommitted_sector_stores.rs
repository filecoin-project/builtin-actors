use cid::Cid;
use fil_actor_miner::State;
use fil_actor_miner::{SectorPreCommitInfo, SectorPreCommitOnChainInfo};
use fil_actors_runtime::test_utils::make_sealed_cid;
use fil_actors_runtime::test_utils::MockRuntime;
use fil_actors_runtime::DealWeight;
use fvm_shared::bigint::Zero;
use fvm_shared::clock::ChainEpoch;
use fvm_shared::econ::TokenAmount;
use fvm_shared::sector::RegisteredSealProof;
use fvm_shared::sector::SectorNumber;

mod util;
use util::*;

#[test]
fn put_get_and_delete() {
    let h = ActorHarness::new(0);
    let mut rt = h.new_runtime();
    h.construct_and_verify(&mut rt);

    let pc1 = new_pre_commit_on_chain(1, make_sealed_cid("1".as_bytes()), TokenAmount::from(1), 1);
    let mut st = h.get_state(&rt);
    st.put_precommitted_sectors(&rt.store, vec![pc1.clone()]).unwrap();
    rt.replace_state(&st);
    assert_eq!(pc1, h.get_precommit(&mut rt, 1));

    let pc2 = new_pre_commit_on_chain(2, make_sealed_cid("2".as_bytes()), TokenAmount::from(1), 1);
    let mut st = h.get_state(&rt);
    st.put_precommitted_sectors(&rt.store, vec![pc2.clone()]).unwrap();
    rt.replace_state(&st);
    assert_eq!(pc2, h.get_precommit(&mut rt, 2));

    let pc3 = new_pre_commit_on_chain(3, make_sealed_cid("2".as_bytes()), TokenAmount::from(1), 1);
    let pc4 = new_pre_commit_on_chain(4, make_sealed_cid("2".as_bytes()), TokenAmount::from(1), 1);
    let mut st = h.get_state(&rt);
    st.put_precommitted_sectors(&rt.store, vec![pc3.clone(), pc4.clone()]).unwrap();
    rt.replace_state(&st);
    assert_eq!(pc3, h.get_precommit(&mut rt, 3));
    assert_eq!(pc4, h.get_precommit(&mut rt, 4));

    delete_pre_commit(&mut rt, 1);
    assert!(!h.has_precommit(&rt, 1));
    assert!(h.has_precommit(&rt, 2));
}

#[test]
fn delete_nonexistent_value_returns_an_error() {
    let h = ActorHarness::new(0);
    let mut rt = h.new_runtime();
    h.construct_and_verify(&mut rt);

    let mut st = h.get_state(&rt);
    assert!(st.delete_precommitted_sectors(&rt.store, &[1]).is_err());
}

#[test]
fn get_nonexistent_value_returns_false() {
    let h = ActorHarness::new(0);
    let mut rt = h.new_runtime();
    h.construct_and_verify(&mut rt);

    assert!(!h.has_precommit(&rt, 1));
}

#[test]
fn duplicate_put_rejected() {
    let h = ActorHarness::new(0);
    let mut rt = h.new_runtime();
    h.construct_and_verify(&mut rt);

    let pc1 = new_pre_commit_on_chain(1, make_sealed_cid("1".as_bytes()), TokenAmount::from(1), 1);

    // In sequence
    let mut st = h.get_state(&rt);
    assert!(st.put_precommitted_sectors(&rt.store, vec![pc1.clone()]).is_ok());
    assert!(st.put_precommitted_sectors(&rt.store, vec![pc1]).is_err());

    // In batch
    let pc2 = new_pre_commit_on_chain(2, make_sealed_cid("2".as_bytes()), TokenAmount::from(1), 1);
    assert!(st.put_precommitted_sectors(&rt.store, vec![pc2.clone(), pc2]).is_err());
}

fn delete_pre_commit(rt: &mut MockRuntime, sector_number: SectorNumber) {
    let mut st = rt.get_state::<State>();
    st.delete_precommitted_sectors(&rt.store, &[sector_number]).unwrap();
    rt.replace_state(&st);
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
    SectorPreCommitOnChainInfo {
        info,
        pre_commit_deposit: deposit,
        pre_commit_epoch: epoch,
        deal_weight: DealWeight::zero(),
        verified_deal_weight: DealWeight::zero(),
    }
}
