use fil_actors_integration_tests::tests::{
    aggregate_bad_sector_number_test, aggregate_bad_sender_test,
    aggregate_one_precommit_expires_test, aggregate_size_limits_test,
    missed_first_post_deadline_test, overdue_precommit_test, skip_sector_test,
    submit_post_succeeds_test,
};
use test_vm::new_test_vm;

#[test]
fn submit_post_succeeds() {
    let v = new_test_vm();
    submit_post_succeeds_test(&*v);
}

#[test]
fn skip_sector() {
    let v = new_test_vm();
    skip_sector_test(&*v);
}

#[test]
fn missed_first_post_deadline() {
    let v = new_test_vm();
    missed_first_post_deadline_test(&*v);
}

#[test]
fn overdue_precommit() {
    let v = new_test_vm();
    overdue_precommit_test(&*v);
}

#[test]
fn aggregate_bad_sector_number() {
    let v = new_test_vm();
    aggregate_bad_sector_number_test(&*v);
}

#[test]
fn aggregate_size_limits() {
    let v = new_test_vm();
    aggregate_size_limits_test(&*v);
}

#[test]
fn aggregate_bad_sender() {
    let v = new_test_vm();
    aggregate_bad_sender_test(&*v);
}

#[test]
fn aggregate_one_precommit_expires() {
    let v = new_test_vm();
    aggregate_one_precommit_expires_test(&*v);
}
