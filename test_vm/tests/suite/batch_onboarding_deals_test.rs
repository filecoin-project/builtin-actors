use fil_actors_integration_tests::tests::{
    batch_onboarding_deals_test, pre_commit_rejects_deal_ids_test,
};
use fil_actors_runtime::test_blockstores::MemoryBlockstore;
use test_vm::TestVM;

#[test]
fn batch_onboarding_deals() {
    let store = MemoryBlockstore::new();
    let v = TestVM::new_with_singletons(store);
    batch_onboarding_deals_test(&v);
}

#[test]
fn pre_commit_rejects_deal_ids() {
    let store = MemoryBlockstore::new();
    let v = TestVM::new_with_singletons(store);
    pre_commit_rejects_deal_ids_test(&v);
}
