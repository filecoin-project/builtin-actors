use fil_actors_integration_tests::tests::create_miner_and_upgrade_sector;
use fil_actors_runtime::runtime::Policy;
use test_case::test_case;
use test_vm::new_test_vm;

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
#[test_case(false; "v1")]
#[test_case(true; "v2")]
fn replica_update_simple_path_success(v2: bool) {
    let v = new_test_vm();
    create_miner_and_upgrade_sector(&*v, v2);
    assert_invariants(&*v, &Policy::default());
}

// Tests a successful upgrade, followed by the sector going faulty and recovering
#[test_case(false; "v1")]
#[test_case(true; "v2")]
fn replica_update_full_path_success(v2: bool) {
    let v = new_test_vm();
    replica_update_full_path_success_test(&*v, v2);
}

#[test_case(false; "v1")]
#[test_case(true; "v2")]
fn upgrade_and_miss_post(v2: bool) {
    let v = new_test_vm();
    upgrade_and_miss_post_test(&*v, v2);
}

#[test]
fn prove_replica_update_multi_dline() {
    let v = new_test_vm();
    prove_replica_update_multi_dline_test(&*v);
}

// ---- Failure cases ----

#[test]
fn immutable_deadline_failure() {
    let v = new_test_vm();
    immutable_deadline_failure_test(&*v);
}

#[test]
fn unhealthy_sector_failure() {
    let v = new_test_vm();
    unhealthy_sector_failure_test(&*v);
}

#[test]
fn terminated_sector_failure() {
    let v = new_test_vm();
    terminated_sector_failure_test(&*v);
}

#[test]
fn bad_batch_size_failure() {
    let v = new_test_vm();
    bad_batch_size_failure_test(&*v);
}

#[test]
fn no_dispute_after_upgrade() {
    let v = new_test_vm();
    nodispute_after_upgrade_test(&*v);
}

#[test]
fn upgrade_bad_post_dispute() {
    let v = new_test_vm();
    upgrade_bad_post_dispute_test(&*v);
}

#[test]
fn bad_post_upgrade_dispute() {
    let v = new_test_vm();
    bad_post_upgrade_dispute_test(&*v);
}

#[test]
fn terminate_after_upgrade() {
    let v = new_test_vm();
    terminate_after_upgrade_test(&*v);
}

#[test]
fn extend_after_upgrade() {
    let v = new_test_vm();
    extend_after_upgrade_test(&*v);
}

#[test]
fn wrong_deadline_index_failure() {
    let v = new_test_vm();

    wrong_deadline_index_failure_test(&*v);
}

#[test]
fn wrong_partition_index_failure() {
    let v = new_test_vm();

    wrong_partition_index_failure_test(&*v);
}

#[test]
fn deal_included_in_multiple_sectors_failure() {
    let v = new_test_vm();
    deal_included_in_multiple_sectors_failure_test(&*v);
}

#[test]
fn replica_update_verified_deal() {
    let v = new_test_vm();

    replica_update_verified_deal_test(&*v);
}

#[test]
fn replica_update_verified_deal_max_term_violated() {
    let v = new_test_vm();
    replica_update_verified_deal_max_term_violated_test(&*v);
}
