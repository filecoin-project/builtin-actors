use fil_actors_integration_tests::tests::prove_replica_update2_test;
use fvm_ipld_blockstore::MemoryBlockstore;
use test_vm::TestVM;

#[test]
fn prove_replica_update2() {
    let store = MemoryBlockstore::new();
    let v = TestVM::<MemoryBlockstore>::new_with_singletons(&store);
    prove_replica_update2_test(&v);
}
