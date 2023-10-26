use fil_actors_integration_tests::tests::{
    commit_sector_with_max_duration_deal_test, extend_legacy_sector_with_deals_test,
    extend_sector_up_to_max_relative_extension_test, extend_updated_sector_with_claims_test,
};
use fil_actors_runtime::test_blockstores::MemoryBlockstore;
use test_vm::TestVM;

#[test]
fn extend_legacy_sector_with_deals() {
    let store = MemoryBlockstore::new();
    let v = TestVM::new_with_singletons(store);
    extend_legacy_sector_with_deals_test(&v, false);
}

#[test]
fn extend2_legacy_sector_with_deals() {
    let store = MemoryBlockstore::new();
    let v = TestVM::new_with_singletons(store);
    extend_legacy_sector_with_deals_test(&v, true);
}

#[test]
fn extend_updated_sector_with_claim() {
    let store = MemoryBlockstore::new();
    let v = TestVM::new_with_singletons(store);
    extend_updated_sector_with_claims_test(&v);
}

#[test]
fn extend_sector_up_to_max_relative_extension() {
    let store = MemoryBlockstore::new();
    let v = TestVM::new_with_singletons(store);
    extend_sector_up_to_max_relative_extension_test(&v);
}

#[test]
fn commit_sector_with_max_duration_deal() {
    let store = MemoryBlockstore::new();
    let v = TestVM::new_with_singletons(store);
    commit_sector_with_max_duration_deal_test(&v);
}
