use fil_actors_integration_tests::tests::{
    missed_first_post_deadline_test, overdue_precommit_test, skip_sector_test,
    submit_post_succeeds_test,
};
use fil_actors_runtime::test_blockstores::MemoryBlockstore;
use test_vm::TestVM;
#[test]
fn submit_post_succeeds() {
    let store = MemoryBlockstore::new();
    let v = TestVM::new_with_singletons(store);
    submit_post_succeeds_test(&v);
}

#[test]
fn skip_sector() {
    let store = MemoryBlockstore::new();
    let v = TestVM::new_with_singletons(store);
    skip_sector_test(&v);
}

#[test]
fn missed_first_post_deadline() {
    let store = MemoryBlockstore::new();
    let v = TestVM::new_with_singletons(store);
    missed_first_post_deadline_test(&v);
}

#[test]
fn overdue_precommit() {
    let store = MemoryBlockstore::new();
    let v = TestVM::new_with_singletons(store);
    overdue_precommit_test(&v);
}
