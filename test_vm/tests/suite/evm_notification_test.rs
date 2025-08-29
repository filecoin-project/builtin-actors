use fil_actors_integration_tests::tests::evm_receives_ddo_notifications_test;
use fil_actors_runtime::test_blockstores::MemoryBlockstore;
use test_vm::TestVM;

/* Test out ddo notifications against a real solidity smart contract */
#[test]
fn evm_notification() {
    let store = MemoryBlockstore::new();
    let v = TestVM::new_with_singletons(store);
    evm_receives_ddo_notifications_test(&v);
}
