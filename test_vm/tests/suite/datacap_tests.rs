use fil_actors_integration_tests::tests::{call_name_symbol_test, datacap_transfer_test};
use fil_actors_runtime::test_blockstores::MemoryBlockstore;
use test_vm::TestVM;

/* Mint a token for client and transfer it to a receiver, exercising error cases */
#[test]
fn datacap_transfer() {
    let store = MemoryBlockstore::new();
    let v = TestVM::new_with_singletons(store);
    datacap_transfer_test(&v);
}

/* Call name & symbol */
#[test]
fn call_name_symbol() {
    let store = MemoryBlockstore::new();
    let v = TestVM::new_with_singletons(store);
    call_name_symbol_test(&v);
}
