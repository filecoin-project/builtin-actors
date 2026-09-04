use fil_actors_integration_tests::tests::{
    add_verifier_via_root_multisig_is_forbidden_test, remove_datacap_disabled_test,
};
use fil_actors_runtime::test_blockstores::MemoryBlockstore;
use test_vm::TestVM;

#[test]
fn add_verifier_via_root_multisig_is_forbidden() {
    let store = MemoryBlockstore::new();
    let v = TestVM::new_with_singletons(store);
    add_verifier_via_root_multisig_is_forbidden_test(&v);
}

#[test]
fn remove_datacap_disabled() {
    let store = MemoryBlockstore::new();
    let v = TestVM::new_with_singletons(store);
    remove_datacap_disabled_test(&v);
}
