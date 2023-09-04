use fil_actors_integration_tests::tests::create_miner_and_upgrade_sector;
use fil_actors_runtime::runtime::Policy;
use fil_actors_runtime::test_blockstores::MemoryBlockstore;
use std::rc::Rc;
use test_vm::TestVM;

use fil_actors_integration_tests::tests::{
    bad_batch_size_failure_test, bad_post_upgrade_dispute_test,
    deal_included_in_multiple_sectors_failure_test, extend_after_upgrade_test,
    immutable_deadline_failure_test, nodispute_after_upgrade_test,
    prove_replica_update_multi_dline_test, replica_update_full_path_success_test,
    replica_update_verified_deal_max_term_violated_test, replica_update_verified_deal_test,
    terminate_after_upgrade_test, terminated_sector_failure_test, unhealthy_sector_failure_test,
    upgrade_and_miss_post_test, upgrade_bad_post_dispute_test, wrong_deadline_index_failure_test,
    wrong_partition_index_failure_test,
};
use fil_actors_integration_tests::util::assert_invariants;

// ---- Success cases ----
// Tests that an active CC sector can be correctly upgraded, and the expected state changes occur
#[test]
fn replica_update_simple_path_success() {
    let store = MemoryBlockstore::new();
    let v = TestVM::new_with_singletons(store);
    create_miner_and_upgrade_sector(&v);
    assert_invariants(&v, &Policy::default(), None);
}

// Tests a successful upgrade, followed by the sector going faulty and recovering
#[test]
fn replica_update_full_path_success() {
    let store = MemoryBlockstore::new();
    let v = TestVM::new_with_singletons(store);
    replica_update_full_path_success_test(&v);
}

#[test]
fn upgrade_and_miss_post() {
    let store = MemoryBlockstore::new();
    let v = TestVM::new_with_singletons(store);
    upgrade_and_miss_post_test(&v);
}

#[test]
fn prove_replica_update_multi_dline() {
    let store = MemoryBlockstore::new();
    let v = TestVM::new_with_singletons(store);
    prove_replica_update_multi_dline_test(&v);
}

// ---- Failure cases ----

#[test]
fn immutable_deadline_failure() {
    let store = MemoryBlockstore::new();
    let v = TestVM::new_with_singletons(store);
    immutable_deadline_failure_test(&v);
}

#[test]
fn unhealthy_sector_failure() {
    let store = MemoryBlockstore::new();
    let v = TestVM::new_with_singletons(store);
    unhealthy_sector_failure_test(&v);
}

#[test]
fn terminated_sector_failure() {
    let store = MemoryBlockstore::new();
    let v = TestVM::new_with_singletons(store);
    terminated_sector_failure_test(&v);
}

#[test]
fn bad_batch_size_failure() {
    let store = MemoryBlockstore::new();
    let v = TestVM::new_with_singletons(store);
    bad_batch_size_failure_test(&v);
}

#[test]
fn no_dispute_after_upgrade() {
    let store = MemoryBlockstore::new();
    let v = TestVM::new_with_singletons(store);
    nodispute_after_upgrade_test(&v);
}

#[test]
fn upgrade_bad_post_dispute() {
    let store = MemoryBlockstore::new();
    let v = TestVM::new_with_singletons(store);
    upgrade_bad_post_dispute_test(&v);
}

#[test]
fn bad_post_upgrade_dispute() {
    let store = MemoryBlockstore::new();
    let v = TestVM::new_with_singletons(store);
    bad_post_upgrade_dispute_test(&v);
}

#[test]
fn terminate_after_upgrade() {
    let store = MemoryBlockstore::new();
    let v = TestVM::new_with_singletons(store);
    terminate_after_upgrade_test(&v);
}

#[test]
fn extend_after_upgrade() {
    let store = MemoryBlockstore::new();
    let v = TestVM::new_with_singletons(store);
    extend_after_upgrade_test(&v);
}

#[test]
fn wrong_deadline_index_failure() {
    let store = MemoryBlockstore::new();
    let v = TestVM::new_with_singletons(store);

    wrong_deadline_index_failure_test(&v);
}

#[test]
fn wrong_partition_index_failure() {
    let store = MemoryBlockstore::new();
    let v = TestVM::new_with_singletons(store);

    wrong_partition_index_failure_test(&v);
}

#[test]
fn deal_included_in_multiple_sectors_failure() {
    let store = MemoryBlockstore::new();
    let v = TestVM::new_with_singletons(store);
    deal_included_in_multiple_sectors_failure_test(&v);
}

#[test]
fn replica_update_verified_deal() {
    let store = MemoryBlockstore::new();
    let v = TestVM::new_with_singletons(store);

    replica_update_verified_deal_test(&v);
}

#[test]
fn replica_update_verified_deal_max_term_violated() {
    let store = Rc::new(MemoryBlockstore::new());
    let v = TestVM::new_with_singletons(store);
    replica_update_verified_deal_max_term_violated_test(&v);
}
