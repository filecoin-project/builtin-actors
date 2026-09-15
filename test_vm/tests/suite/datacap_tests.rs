use fil_actors_integration_tests::tests::{call_name_symbol_test, datacap_mint_disabled_test};
use fil_actors_runtime::test_blockstores::MemoryBlockstore;
use test_vm::TestVM;

/* Mint is deprecated (FIP-0118) and always returns USR_FORBIDDEN */
#[test]
fn datacap_mint_disabled() {
    let store = MemoryBlockstore::new();
    let v = TestVM::new_with_singletons(store);
    datacap_mint_disabled_test(&v);
}

/* Call name & symbol */
#[test]
fn call_name_symbol() {
    let store = MemoryBlockstore::new();
    let v = TestVM::new_with_singletons(store);
    call_name_symbol_test(&v);
}
