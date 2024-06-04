use fil_actors_integration_tests::tests::prove_replica_update2_test;
use fil_actors_runtime::test_blockstores::MemoryBlockstore;
use test_vm::TestVM;

#[test]
fn prove_replica_update2() {
    let store = MemoryBlockstore::new();
    let v = TestVM::new_with_singletons(store);
    prove_replica_update2_test(&v);
}
