use fil_actors_integration_tests::tests::evm_sector_status_test;
use fil_actors_runtime::test_blockstores::MemoryBlockstore;
use test_vm::TestVM;

#[test]
fn evm_sector_status() {
    let store = MemoryBlockstore::new();
    let v = TestVM::new_with_singletons(store);
    evm_sector_status_test(&v);
}
