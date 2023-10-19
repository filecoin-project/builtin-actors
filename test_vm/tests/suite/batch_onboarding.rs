use fil_actors_integration_tests::tests::batch_onboarding_test;
use fil_actors_runtime::test_blockstores::MemoryBlockstore;
use std::rc::Rc;
use test_case::test_case;
use test_vm::TestVM;

// Test for batch pre-commit and aggregate prove-commit onboarding sectors (no deals).
#[test_case(false; "v1")]
#[test_case(true; "v2")]
fn batch_onboarding(v2: bool) {
    let store = MemoryBlockstore::new();
    let v = TestVM::new_with_singletons(Rc::new(store));

    batch_onboarding_test(&v, v2);
}
