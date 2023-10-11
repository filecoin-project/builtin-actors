use fil_actor_blockstore::MemoryBlockstore;
use fil_actors_integration_tests::tests::placeholder_deploy_test;
use test_vm::TestVM;

#[test]
fn placeholder_deploy() {
    let store = MemoryBlockstore::new();
    let v = TestVM::<MemoryBlockstore>::new_with_singletons(&store);

    placeholder_deploy_test(&v);
}
