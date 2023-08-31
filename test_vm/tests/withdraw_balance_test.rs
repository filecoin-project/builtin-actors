use fil_actors_integration_tests::tests::{
    withdraw_balance_fail_test, withdraw_balance_success_test,
};
use test_vm::new_test_vm;

#[test]
fn withdraw_balance_success() {
    let v = new_test_vm();
    withdraw_balance_success_test(&*v);
}

#[test]
fn withdraw_balance_fail() {
    let v = new_test_vm();
    withdraw_balance_fail_test(&*v);
}
