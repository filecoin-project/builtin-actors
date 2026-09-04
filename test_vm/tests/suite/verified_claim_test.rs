use fil_actors_integration_tests::tests::{
    unactivated_verified_deal_expires_test, verified_deal_sector_lifecycle_test,
    verified_deals_commit_without_claims_test,
};
use fil_actors_runtime::test_blockstores::MemoryBlockstore;
use test_vm::TestVM;

#[test]
fn verified_deal_sector_lifecycle() {
    let store = MemoryBlockstore::new();
    let v = TestVM::new_with_singletons(store);
    verified_deal_sector_lifecycle_test(&v);
}

#[test]
fn unactivated_verified_deal_expires() {
    let store = MemoryBlockstore::new();
    let v = TestVM::new_with_singletons(store);
    unactivated_verified_deal_expires_test(&v);
}

#[test]
fn verified_deals_commit_without_claims() {
    let store = MemoryBlockstore::new();
    let v = TestVM::new_with_singletons(store);
    verified_deals_commit_without_claims_test(&v);
}
