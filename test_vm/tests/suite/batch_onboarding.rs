use fil_actors_integration_tests::tests::batch_onboarding_test;
use fil_actors_runtime::test_blockstores::TrackingMemBlockstore;
use test_case::test_case;
use test_vm::TestVM;

// Test for batch pre-commit and aggregate prove-commit onboarding sectors (no deals).
#[test_case(false; "v1")]
#[test_case(true; "v2")]
fn batch_onboarding(v2: bool) {
    let store = TrackingMemBlockstore::new();
    let v = TestVM::<TrackingMemBlockstore>::new_with_singletons(&store);

    batch_onboarding_test(&v, v2);
}
