use fil_actors_integration_tests::tests::{
    commit_sector_with_max_duration_deal_test, extend_legacy_sector_with_deals_test,
    extend_sector_up_to_max_relative_extension_test, extend_updated_sector_with_claims_test,
};
use test_vm::new_test_vm;

#[test]
fn extend_legacy_sector_with_deals() {
    let v = new_test_vm();
    extend_legacy_sector_with_deals_test(&*v, false);
}

#[test]
fn extend2_legacy_sector_with_deals() {
    let v = new_test_vm();
    extend_legacy_sector_with_deals_test(&*v, true);
}

#[test]
fn extend_updated_sector_with_claim() {
    let v = new_test_vm();
    extend_updated_sector_with_claims_test(&*v);
}

#[test]
fn extend_sector_up_to_max_relative_extension() {
    let v = new_test_vm();
    extend_sector_up_to_max_relative_extension_test(&*v);
}

#[test]
fn commit_sector_with_max_duration_deal() {
    let v = new_test_vm();
    commit_sector_with_max_duration_deal_test(&*v);
}
