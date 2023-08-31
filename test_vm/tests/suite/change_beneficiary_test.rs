use fil_actors_integration_tests::tests::{
    change_beneficiary_back_owner_success_test, change_beneficiary_fail_test,
    change_beneficiary_success_test,
};
use fvm_ipld_blockstore::MemoryBlockstore;
use test_vm::TestVM;

#[test]
fn change_beneficiary_success() {
    let store = MemoryBlockstore::new();
    let v = TestVM::<MemoryBlockstore>::new_with_singletons(&store);
    change_beneficiary_success_test(&v);
}

#[test]
fn change_beneficiary_back_owner_success() {
    let store = MemoryBlockstore::new();
    let v = TestVM::<MemoryBlockstore>::new_with_singletons(&store);
    change_beneficiary_back_owner_success_test(&v);
}

#[test]
fn change_beneficiary_fail() {
    let store = MemoryBlockstore::new();
    let v = TestVM::<MemoryBlockstore>::new_with_singletons(&store);
    change_beneficiary_fail_test(&v);
}
