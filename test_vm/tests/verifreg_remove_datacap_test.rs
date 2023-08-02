use fil_actors_integration_tests::tests::{
    remove_datacap_fails_on_verifreg_test, remove_datacap_simple_successful_path_test,
};
use fvm_ipld_blockstore::MemoryBlockstore;
use test_vm::TestVM;

#[test]
fn remove_datacap_simple_successful_path() {
    let store = MemoryBlockstore::new();
    let v = TestVM::<MemoryBlockstore>::new_with_singletons(&store);
    remove_datacap_simple_successful_path_test(&v);
}

#[test]
fn remove_datacap_fails_on_verifreg() {
    let store = MemoryBlockstore::new();
    let v = TestVM::<MemoryBlockstore>::new_with_singletons(&store);
    remove_datacap_fails_on_verifreg_test(&v);
}
