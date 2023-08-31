use fil_actors_integration_tests::tests::placeholder_deploy_test;
use test_vm::new_test_vm;

#[test]
fn placeholder_deploy() {
    let v = new_test_vm();

    placeholder_deploy_test(&*v);
}
