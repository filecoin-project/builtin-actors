use fil_actors_integration_tests::tests::{
    aggregate_bad_sector_number_test, aggregate_bad_sender_test,
    aggregate_one_precommit_expires_test, aggregate_size_limits_test,
    missed_first_post_deadline_test, overdue_precommit_test, skip_sector_test,
    submit_post_succeeds_test,
};
use fil_actors_runtime::test_blockstores::TrackingMemBlockstore;
use test_vm::TestVM;

#[test]
fn submit_post_succeeds() {
    let store = TrackingMemBlockstore::new();
    let v = TestVM::new_with_singletons(&store);
    submit_post_succeeds_test(&v);
}

#[test]
fn skip_sector() {
    let store = TrackingMemBlockstore::new();
    let v = TestVM::new_with_singletons(&store);
    skip_sector_test(&v);
}

#[test]
fn missed_first_post_deadline() {
    let store = TrackingMemBlockstore::new();
    let v = TestVM::new_with_singletons(&store);
    missed_first_post_deadline_test(&v);
}

#[test]
fn overdue_precommit() {
    let store = TrackingMemBlockstore::new();
    let v = TestVM::new_with_singletons(&store);
    overdue_precommit_test(&v);
}

#[test]
fn aggregate_bad_sector_number() {
    let store = TrackingMemBlockstore::new();
    let v = TestVM::new_with_singletons(&store);
    aggregate_bad_sector_number_test(&v);
}

#[test]
fn aggregate_size_limits() {
    let store = TrackingMemBlockstore::new();
    let v = TestVM::new_with_singletons(&store);
    aggregate_size_limits_test(&v);
}

#[test]
fn aggregate_bad_sender() {
    let store = TrackingMemBlockstore::new();
    let v = TestVM::new_with_singletons(&store);
    aggregate_bad_sender_test(&v);
}

#[test]
fn aggregate_one_precommit_expires() {
    let store = TrackingMemBlockstore::new();
    let v = TestVM::new_with_singletons(&store);
    aggregate_one_precommit_expires_test(&v);
}
