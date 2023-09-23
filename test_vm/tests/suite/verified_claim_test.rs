use fil_actors_integration_tests::tests::{
    deal_passes_claim_fails_test, expired_allocations_test, verified_claim_scenario_test,
};
use fvm_ipld_blockstore::MemoryBlockstore;
use test_vm::TestVM;

// Tests a scenario involving a verified deal from the built-in market, with associated
// allocation and claim.
// This test shares some set-up copied from extend_sectors_test.
#[test]
fn verified_claim_scenario() {
    let store = MemoryBlockstore::new();
    let v = TestVM::<MemoryBlockstore>::new_with_singletons(&store);
    verified_claim_scenario_test(&v);
}

#[test]
fn expired_allocations() {
    let store = MemoryBlockstore::new();
    let v = TestVM::<MemoryBlockstore>::new_with_singletons(&store);
    expired_allocations_test(&v);
}

#[test]
fn deal_passes_claim_fails() {
    let store = MemoryBlockstore::new();
    let v = TestVM::<MemoryBlockstore>::new_with_singletons(&store);
    deal_passes_claim_fails_test(&v);
}
