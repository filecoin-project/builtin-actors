use fil_actors_integration_tests::tests::{
    proposal_hash_test, swap_self_1_of_2_test, swap_self_2_of_3_test, test_delete_self_inner_test,
};
use fil_actors_runtime::test_blockstores::MemoryBlockstore;
use test_vm::TestVM;

#[test]
fn proposal_hash() {
    let store = MemoryBlockstore::new();
    let v = TestVM::new_with_singletons(store);
    proposal_hash_test(&v);
}

#[test]
fn test_delete_self() {
    let test = |threshold: usize, signers: u64, remove_idx: usize| {
        let store = MemoryBlockstore::new();
        let v = TestVM::new_with_singletons(store);
        test_delete_self_inner_test(&v, signers, threshold, remove_idx);
    };
    test(2, 3, 0); // 2 of 3 removed is proposer
    test(2, 3, 1); // 2 of 3 removed is approver
    test(2, 2, 0); // 2 of 2 removed is proposer
    test(1, 2, 0); // 1 of 2
}

#[test]
fn swap_self_1_of_2() {
    let store = MemoryBlockstore::new();
    let v = TestVM::new_with_singletons(store);
    swap_self_1_of_2_test(&v);
}

#[test]
fn swap_self_2_of_3() {
    let store = MemoryBlockstore::new();
    let v = TestVM::new_with_singletons(store);
    swap_self_2_of_3_test(&v);
}
