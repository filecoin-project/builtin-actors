use fil_actors_integration_tests::tests::{
    change_beneficiary_back_owner_success_test, change_beneficiary_fail_test,
    change_beneficiary_success_test,
};
use test_vm::new_test_vm;

#[test]
fn change_beneficiary_success() {
    let v = new_test_vm();
    change_beneficiary_success_test(&*v);
}

#[test]
fn change_beneficiary_back_owner_success() {
    let v = new_test_vm();
    change_beneficiary_back_owner_success_test(&*v);
}

#[test]
fn change_beneficiary_fail() {
    let v = new_test_vm();
    change_beneficiary_fail_test(&*v);
}
