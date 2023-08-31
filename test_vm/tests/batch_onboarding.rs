use fil_actors_integration_tests::tests::batch_onboarding_test;
use test_case::test_case;
use test_vm::new_test_vm;

// Test for batch pre-commit and aggregate prove-commit onboarding sectors (no deals).
#[test_case(false; "v1")]
#[test_case(true; "v2")]
fn batch_onboarding(v2: bool) {
    let v = new_test_vm();

    batch_onboarding_test(&*v, v2);
}
