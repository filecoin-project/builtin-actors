use fil_actors_integration_tests::tests::{
    deal_passes_claim_fails_test, expired_allocations_test, verified_claim_scenario_test,
};
use test_vm::new_test_vm;

// Tests a scenario involving a verified deal from the built-in market, with associated
// allocation and claim.
// This test shares some set-up copied from extend_sectors_test.
#[test]
fn verified_claim_scenario() {
    let v = new_test_vm();
    verified_claim_scenario_test(&*v);
}

#[test]
fn expired_allocations() {
    let v = new_test_vm();
    expired_allocations_test(&*v);
}

#[test]
fn deal_passes_claim_fails() {
    let v = new_test_vm();
    deal_passes_claim_fails_test(&*v);
}
