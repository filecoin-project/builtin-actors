use fil_actors_integration_tests::tests::withdraw_balance_success_test;
use fvm_ipld_blockstore::MemoryBlockstore;
use test_vm::TestVM;

#[test]
fn withdraw_balance_success() {
    let store = MemoryBlockstore::new();
    let v = TestVM::<MemoryBlockstore>::new_with_singletons(&store);
    withdraw_balance_success_test(&v);
}
