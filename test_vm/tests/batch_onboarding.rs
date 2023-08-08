use fil_actors_integration_tests::tests::batch_onboarding_test;
use fvm_ipld_blockstore::MemoryBlockstore;
use test_vm::TestVM;

// Test for batch pre-commit and aggregate prove-commit onboarding sectors (no deals).
#[test]
fn batch_onboarding() {
    let store = MemoryBlockstore::new();
    let v = TestVM::<MemoryBlockstore>::new_with_singletons(&store);

    batch_onboarding_test(&v);
}
