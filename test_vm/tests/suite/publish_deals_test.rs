use fil_actors_integration_tests::tests::{
    all_deals_are_good_test, psd_all_deals_are_bad_test, psd_bad_piece_size_test, psd_bad_sig_test,
    psd_client_address_cannot_be_resolved_test, psd_deal_duration_too_long_test,
    psd_duplicate_deal_in_batch_test, psd_duplicate_deal_in_state_test,
    psd_mismatched_provider_test, psd_no_client_lockup_test,
    psd_not_enough_client_lockup_for_batch_test, psd_not_enough_provider_lockup_for_batch_test,
    psd_random_assortment_of_failures_test, psd_start_time_in_past_test,
    psd_valid_deals_with_ones_longer_than_540_test, psd_verified_deal_fails_getting_datacap_test,
};
use fil_actors_runtime::test_blockstores::MemoryBlockstore;
use test_vm::TestVM;

#[test]
fn psd_mismatched_provider() {
    let store = MemoryBlockstore::new();
    let v = TestVM::new_with_singletons(store);
    psd_mismatched_provider_test(&v);
}

#[test]
fn psd_bad_piece_size() {
    let store = MemoryBlockstore::new();
    let v = TestVM::new_with_singletons(store);
    psd_bad_piece_size_test(&v);
}

#[test]
fn psd_start_time_in_past() {
    let store = MemoryBlockstore::new();
    let v = TestVM::new_with_singletons(store);
    psd_start_time_in_past_test(&v);
}

#[test]
fn psd_client_address_cannot_be_resolved() {
    let store = MemoryBlockstore::new();
    let v = TestVM::new_with_singletons(store);
    psd_client_address_cannot_be_resolved_test(&v);
}

#[test]
fn psd_no_client_lockup() {
    let store = MemoryBlockstore::new();
    let v = TestVM::new_with_singletons(store);
    psd_no_client_lockup_test(&v);
}

#[test]
fn psd_not_enough_client_lockup_for_batch() {
    let store = MemoryBlockstore::new();
    let v = TestVM::new_with_singletons(store);
    psd_not_enough_client_lockup_for_batch_test(&v);
}

#[test]
fn psd_not_enough_provider_lockup_for_batch() {
    let store = MemoryBlockstore::new();
    let v = TestVM::new_with_singletons(store);
    psd_not_enough_provider_lockup_for_batch_test(&v);
}

#[test]
fn psd_duplicate_deal_in_batch() {
    let store = MemoryBlockstore::new();
    let v = TestVM::new_with_singletons(store);
    psd_duplicate_deal_in_batch_test(&v);
}

#[test]
fn psd_duplicate_deal_in_state() {
    let store = MemoryBlockstore::new();
    let v = TestVM::new_with_singletons(store);
    psd_duplicate_deal_in_state_test(&v);
}

#[test]
fn psd_verified_deal_fails_getting_datacap() {
    let store = MemoryBlockstore::new();
    let v = TestVM::new_with_singletons(store);
    psd_verified_deal_fails_getting_datacap_test(&v);
}

#[test]
fn psd_random_assortment_of_failures() {
    let store = MemoryBlockstore::new();
    let v = TestVM::new_with_singletons(store);
    psd_random_assortment_of_failures_test(&v);
}

#[test]
fn psd_all_deals_are_bad() {
    let store = MemoryBlockstore::new();
    let v = TestVM::new_with_singletons(store);
    psd_all_deals_are_bad_test(&v);
}

#[test]
fn psd_bad_sig() {
    let store = MemoryBlockstore::new();
    let v = TestVM::new_with_singletons(store);
    psd_bad_sig_test(&v);
}

#[test]
fn psd_all_deals_are_good() {
    let store = MemoryBlockstore::new();
    let v = TestVM::new_with_singletons(store);
    all_deals_are_good_test(&v);
}

#[test]
fn psd_valid_deals_with_ones_longer_than_540() {
    let store = MemoryBlockstore::new();
    let v = TestVM::new_with_singletons(store);
    psd_valid_deals_with_ones_longer_than_540_test(&v);
}

#[test]
fn psd_deal_duration_too_long() {
    let store = MemoryBlockstore::new();
    let v = TestVM::new_with_singletons(store);
    psd_deal_duration_too_long_test(&v);
}
