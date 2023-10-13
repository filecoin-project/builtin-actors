use fil_actors_integration_tests::tests::account_authenticate_message_test;
use fil_actors_runtime::test_blockstores::TrackingMemBlockstore;
use test_vm::TestVM;

#[test]
fn account_authenticate_message() {
    let store = TrackingMemBlockstore::new();
    let v = TestVM::<TrackingMemBlockstore>::new_with_singletons(&store);
    account_authenticate_message_test(&v);
}
