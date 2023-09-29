use fil_actors_integration_tests::tests::batch_onboarding_deals_test;
use fvm_ipld_blockstore::MemoryBlockstore;
use test_vm::TestVM;

#[test]
fn batch_onboarding_deals() {
    let store = MemoryBlockstore::new();
    let v = TestVM::<MemoryBlockstore>::new_with_singletons(&store);
    batch_onboarding_deals_test(&v);
}
