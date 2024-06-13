use fil_actors_integration_tests::tests::{
    prove_commit_sectors_aggregate_niporep_test, prove_commit_sectors_niporep_test,
};
use fil_actors_runtime::test_blockstores::MemoryBlockstore;
use test_vm::TestVM;

#[test_log::test]
fn prove_commit_sectors_niporep() {
    let store = MemoryBlockstore::new();
    let v = TestVM::new_with_singletons(store);
    prove_commit_sectors_niporep_test(&v);
}

#[test_log::test]
fn prove_commit_sectors_aggregate_niporep() {
    let store = MemoryBlockstore::new();
    let v = TestVM::new_with_singletons(store);
    prove_commit_sectors_aggregate_niporep_test(&v);
}
