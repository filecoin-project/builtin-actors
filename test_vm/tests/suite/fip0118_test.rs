use fil_actors_integration_tests::tests::{
    legacy_cc_sector_reaches_10x_by_snapping_test,
    legacy_sector_termination_removes_its_own_power_test, new_cc_sector_gets_10x_test,
    ni_sector_gets_10x_test, simple_qap_sector_extends_without_power_change_test,
    verified_deal_gets_10x_without_datacap_test, verifreg_minting_disabled_test,
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
fn verified_deal_gets_10x_without_datacap() {
    let store = MemoryBlockstore::new();
    let v = TestVM::new_with_singletons(store);
    verified_deal_gets_10x_without_datacap_test(&v);
}

#[test]
fn legacy_cc_sector_reaches_10x_by_snapping() {
    let store = MemoryBlockstore::new();
    let v = TestVM::new_with_singletons(store);
    legacy_cc_sector_reaches_10x_by_snapping_test(&v);
}

#[test]
fn legacy_sector_termination_removes_its_own_power() {
    let store = MemoryBlockstore::new();
    let v = TestVM::new_with_singletons(store);
    legacy_sector_termination_removes_its_own_power_test(&v);
}

#[test]
fn simple_qap_sector_extends_without_power_change() {
    let store = MemoryBlockstore::new();
    let v = TestVM::new_with_singletons(store);
    simple_qap_sector_extends_without_power_change_test(&v);
}
