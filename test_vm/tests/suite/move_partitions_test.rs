use fil_actors_integration_tests::tests::move_partitions_test;
use fvm_ipld_blockstore::MemoryBlockstore;
use test_vm::TestVM;

#[test]
fn move_partitions() {
    let store = MemoryBlockstore::new();
    let v = TestVM::<MemoryBlockstore>::new_with_singletons(&store);
    move_partitions_test(&v);
}
