use fil_actors_integration_tests::tests::terminate_sectors_test;
use fil_actors_runtime::test_blockstores::TrackingMemBlockstore;
use test_vm::TestVM;

#[test]
fn terminate_sectors() {
    let store = TrackingMemBlockstore::new();
    let v = TestVM::<TrackingMemBlockstore>::new_with_singletons(&store);
    terminate_sectors_test(&v);
}
