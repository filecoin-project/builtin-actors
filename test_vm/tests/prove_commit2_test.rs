use fil_actors_integration_tests::tests::prove_commit_sectors2_test;
use fvm_ipld_blockstore::MemoryBlockstore;
use test_vm::TestVM;

#[test]
fn prove_commit_sectors2() {
    let store = MemoryBlockstore::new();
    let v = TestVM::<MemoryBlockstore>::new_with_singletons(&store);
    prove_commit_sectors2_test(&v);
}
