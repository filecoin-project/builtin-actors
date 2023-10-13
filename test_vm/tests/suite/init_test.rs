use fil_actors_integration_tests::tests::placeholder_deploy_test;
use fil_actors_runtime::test_blockstores::TrackingMemBlockstore;
use test_vm::TestVM;

#[test]
fn placeholder_deploy() {
    let store = TrackingMemBlockstore::new();
    let v = TestVM::new_with_singletons(&store);

    placeholder_deploy_test(&v);
}
