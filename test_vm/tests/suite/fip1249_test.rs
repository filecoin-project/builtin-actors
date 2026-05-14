use fil_actors_integration_tests::tests::{
    new_cc_sector_gets_10x_test, ni_sector_gets_10x_test, verified_deal_no_datacap_ops_test,
    verifreg_minting_disabled_test,
};
use fil_actors_runtime::test_blockstores::MemoryBlockstore;
use test_vm::TestVM;

#[test]
fn new_cc_sector_gets_10x() {
    let store = MemoryBlockstore::new();
    let v = TestVM::new_with_singletons(store);
    new_cc_sector_gets_10x_test(&v);
}

#[test]
fn ni_sector_gets_10x() {
    let store = MemoryBlockstore::new();
    let v = TestVM::new_with_singletons(store);
    ni_sector_gets_10x_test(&v);
}

#[test]
fn verifreg_minting_disabled() {
    let store = MemoryBlockstore::new();
    let v = TestVM::new_with_singletons(store);
    verifreg_minting_disabled_test(&v);
}

#[test]
fn verified_deal_no_datacap_ops() {
    let store = MemoryBlockstore::new();
    let v = TestVM::new_with_singletons(store);
    verified_deal_no_datacap_ops_test(&v);
}
