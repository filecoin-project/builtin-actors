use fil_actor_blockstore::MemoryBlockstore;
use fil_actors_integration_tests::tests::account_authenticate_message_test;
use test_vm::TestVM;

#[test]
fn account_authenticate_message() {
    let store = MemoryBlockstore::new();
    let v = TestVM::<MemoryBlockstore>::new_with_singletons(&store);
    account_authenticate_message_test(&v);
}
