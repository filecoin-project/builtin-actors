use fil_actors_integration_tests::tests::terminate_sectors_test;
use test_vm::new_test_vm;

#[test]
fn terminate_sectors() {
    let v = new_test_vm();
    terminate_sectors_test(&*v);
}
