use fil_actors_integration_tests::tests::batch_onboarding_deals_test;
use test_vm::new_test_vm;

#[test]
fn batch_onboarding_deals() {
    let v = new_test_vm();
    batch_onboarding_deals_test(&*v);
}
