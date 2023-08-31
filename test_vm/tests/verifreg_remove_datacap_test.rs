use fil_actors_integration_tests::tests::{
    remove_datacap_fails_on_verifreg_test, remove_datacap_simple_successful_path_test,
};
use test_vm::new_test_vm;

#[test]
fn remove_datacap_simple_successful_path() {
    let v = new_test_vm();
    remove_datacap_simple_successful_path_test(&*v);
}

#[test]
fn remove_datacap_fails_on_verifreg() {
    let v = new_test_vm();
    remove_datacap_fails_on_verifreg_test(&*v);
}
