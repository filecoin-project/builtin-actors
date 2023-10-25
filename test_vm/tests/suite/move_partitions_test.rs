use fil_actors_integration_tests::tests::move_partitions_test;
use fil_actors_runtime::test_blockstores::MemoryBlockstore;
use test_vm::TestVM;

#[test]
fn move_partitions() {
    let store = MemoryBlockstore::new();
    let v = TestVM::new_with_singletons(store);
    move_partitions_test(&v);
}
