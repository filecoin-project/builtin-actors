use fil_actors_integration_tests::tests::account_authenticate_message_test;
use test_vm::new_test_vm;

#[test]
fn account_authenticate_message() {
    let v = new_test_vm();
    account_authenticate_message_test(&*v);
}
