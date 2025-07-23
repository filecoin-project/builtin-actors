use fil_actors_integration_tests::tests::{
    test_multisig_as_verifreg_root_addverifier,
    test_multisig_as_verifreg_root_addverifier_fails_without_threshold,
};
use fil_actors_runtime::test_blockstores::MemoryBlockstore;
use test_vm::TestVM;

#[test]
fn multisig_as_verifreg_root_addverifier() {
    let store = MemoryBlockstore::new();
    let v = TestVM::new_with_singletons(store);
    test_multisig_as_verifreg_root_addverifier(&v);
}

#[test]
fn multisig_as_verifreg_root_addverifier_fails_without_threshold() {
    let store = MemoryBlockstore::new();
    let v = TestVM::new_with_singletons(store);
    test_multisig_as_verifreg_root_addverifier_fails_without_threshold(&v);
}
