use fil_actors_integration_tests::tests::{
    reward_f02_award_and_claim, reward_f02_queued_apply_and_drop,
};
use fil_actors_runtime::test_blockstores::MemoryBlockstore;
use test_vm::TestVM;

#[test]
fn award_and_claim() {
    reward_f02_award_and_claim(&TestVM::new_with_singletons(MemoryBlockstore::new()));
}

#[test]
fn queued_apply_and_drop() {
    reward_f02_queued_apply_and_drop(&TestVM::new_with_singletons(MemoryBlockstore::new()));
}
