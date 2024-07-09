use fil_actors_integration_tests::tests::{
    prove_commit_ni_next_deadline_post_required_test,
    prove_commit_ni_partial_success_not_required_test, prove_commit_ni_whole_success_test,
};
use fil_actors_runtime::test_blockstores::MemoryBlockstore;
use test_vm::TestVM;

#[test]
fn prove_commit_ni_whole_success() {
    let store = MemoryBlockstore::new();
    let v = TestVM::new_with_singletons(store);
    prove_commit_ni_whole_success_test(&v);
}

#[test]
fn prove_commit_ni_partial_success_not_required() {
    let store = MemoryBlockstore::new();
    let v = TestVM::new_with_singletons(store);
    prove_commit_ni_partial_success_not_required_test(&v);
}

#[test]
fn prove_commit_ni_next_deadline_post_required() {
    let store = MemoryBlockstore::new();
    let v = TestVM::new_with_singletons(store);
    prove_commit_ni_next_deadline_post_required_test(&v);
}
