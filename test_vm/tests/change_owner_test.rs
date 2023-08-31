use fil_actors_integration_tests::tests::{
    change_owner_fail_test, change_owner_success_test, keep_beneficiary_when_owner_changed_test,
};
use test_vm::new_test_vm;

#[test]
fn change_owner_success() {
    let v = new_test_vm();
    change_owner_success_test(&*v);
}

#[test]
fn keep_beneficiary_when_owner_changed() {
    let v = new_test_vm();
    keep_beneficiary_when_owner_changed_test(&*v);
}

#[test]
fn change_owner_fail() {
    let v = new_test_vm();
    change_owner_fail_test(&*v);
}
