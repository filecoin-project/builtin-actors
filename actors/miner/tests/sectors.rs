use fil_actor_miner::SectorOnChainInfo;
use fil_actor_miner::Sectors;
use fil_actors_runtime::test_utils::*;
use fvm_ipld_bitfield::BitField;
use fvm_ipld_blockstore::MemoryBlockstore;
use fvm_shared::sector::RegisteredSealProof;

mod util;
use crate::util::sectors_arr;

fn make_sector(i: u64) -> SectorOnChainInfo {
    SectorOnChainInfo {
        sector_number: i,
        seal_proof: RegisteredSealProof::StackedDRG32GiBV1P1,
        sealed_cid: make_sealed_cid(format!("commR-{}", i).as_bytes()),
        ..Default::default()
    }
}

fn setup_sectors(store: &'_ MemoryBlockstore) -> Sectors<'_, MemoryBlockstore> {
    sectors_arr(store, vec![make_sector(0), make_sector(1), make_sector(5)])
}

fn bf_from_vec(vec: Vec<u64>) -> BitField {
    let mut bf = BitField::new();
    for item in vec {
        bf.set(item)
    }
    bf
}

#[test]
fn loads_sectors() {
    let store = MemoryBlockstore::default();
    let sectors = setup_sectors(&store);

    let mut bf = bf_from_vec(vec![0, 5]);
    let vec_sectors = sectors.load_sector(&bf).unwrap();
    assert_eq!(vec_sectors.len(), 2);
    assert_eq!(make_sector(0), vec_sectors[0]);
    assert_eq!(make_sector(5), vec_sectors[1]);

    bf = bf_from_vec(vec![0, 3]);
    let res = sectors.load_sector(&bf);
    assert!(res.is_err());
}

#[test]
fn stores_sectors() {
    let store = MemoryBlockstore::default();
    let mut sectors = setup_sectors(&store);

    let s0 = make_sector(0);
    let mut s1 = make_sector(1);
    s1.activation = 1;

    let s3 = make_sector(3);
    let s5 = make_sector(5);

    sectors.store(vec![s3.clone(), s1.clone()]).unwrap();

    let bf = bf_from_vec(vec![0, 1, 3, 5]);
    let vec_sectors = sectors.load_sector(&bf).unwrap();
    assert_eq!(vec_sectors.len(), 4);
    assert_eq!(&s0, &vec_sectors[0]);
    assert_eq!(&s1, &vec_sectors[1]);
    assert_eq!(1, vec_sectors[1].activation.clone());
    assert_eq!(&s3, &vec_sectors[2]);
    assert_eq!(&s5, &vec_sectors[3]);
}

#[test]
fn loads_and_stores_no_sectors() {
    let store = MemoryBlockstore::default();
    let mut sectors = setup_sectors(&store);

    let bf = bf_from_vec(vec![]);
    let vec_sectors = sectors.load_sector(&bf).unwrap();
    assert_eq!(vec_sectors.len(), 0);
    sectors.store(vec![]).unwrap();
}

#[test]
fn gets_sectors() {
    let store = MemoryBlockstore::default();
    let sectors = setup_sectors(&store);

    let s0_option = sectors.get(0).unwrap();
    assert!(s0_option.is_some());
    assert_eq!(make_sector(0), s0_option.unwrap());

    let res = sectors.get(3);
    assert!(res.is_ok());
    assert!(res.unwrap().is_none());
}

#[test]
fn must_get() {
    let store = MemoryBlockstore::default();
    let sectors = setup_sectors(&store);

    let s0 = sectors.must_get(0).unwrap();
    assert_eq!(make_sector(0), s0);

    let res = sectors.must_get(3);
    assert!(res.is_err());
}

#[test]
fn loads_for_proof_with_replacement() {
    let store = MemoryBlockstore::default();
    let sectors = setup_sectors(&store);

    let s1 = make_sector(1);
    let vec_sectors =
        sectors.load_for_proof(&bf_from_vec(vec![0, 1]), &bf_from_vec(vec![0])).unwrap();
    assert_eq!(s1, vec_sectors[0]);
    assert_eq!(s1, vec_sectors[1]);
}

#[test]
fn loads_for_proof_without_replacement() {
    let store = MemoryBlockstore::default();
    let sectors = setup_sectors(&store);

    let s0 = make_sector(0);
    let s1 = make_sector(1);

    let vec_sectors = sectors.load_for_proof(&bf_from_vec(vec![0, 1]), &BitField::new()).unwrap();
    assert_eq!(s0, vec_sectors[0]);
    assert_eq!(s1, vec_sectors[1]);
}

#[test]
fn empty_proof() {
    let store = MemoryBlockstore::default();
    let sectors = setup_sectors(&store);

    let vec_sectors = sectors.load_for_proof(&BitField::new(), &BitField::new()).unwrap();
    assert_eq!(vec_sectors.len(), 0);
}

#[test]
fn no_non_faulty_sectors() {
    let store = MemoryBlockstore::default();
    let sectors = setup_sectors(&store);

    let vec_sectors = sectors.load_for_proof(&bf_from_vec(vec![1]), &bf_from_vec(vec![1])).unwrap();
    assert_eq!(vec_sectors.len(), 0);
}
