use fil_actors_integration_tests::tests::{
    bad_post_upgrade_dispute_test, create_miner_and_upgrade_sector,
    deal_included_in_multiple_sectors_failure_test, extend_after_upgrade_test,
    immutable_deadline_failure_test, nodispute_after_upgrade_test,
    prove_replica_update_multi_dline_test, prove_replica_update2_test,
    replica_update_full_path_success_test, terminate_after_upgrade_test,
    terminated_sector_failure_test, unhealthy_sector_failure_test, upgrade_and_miss_post_test,
    upgrade_bad_post_dispute_test, wrong_deadline_index_failure_test,
    wrong_partition_index_failure_test,
};
use fil_actors_integration_tests::util::assert_invariants;
use fil_actors_runtime::runtime::Policy;
use fil_actors_runtime::test_blockstores::MemoryBlockstore;
use test_vm::TestVM;

macro_rules! replica_update_tests {
    ($($name:ident => $scenario:ident,)*) => {
        $(
            #[test]
            fn $name() {
                let store = MemoryBlockstore::new();
                let v = TestVM::new_with_singletons(store);
                $scenario(&v);
            }
        )*
    };
}

// ---- Success cases ----
#[test]
fn replica_update_simple_path_success() {
    let store = MemoryBlockstore::new();
    let v = TestVM::new_with_singletons(store);
    create_miner_and_upgrade_sector(&v);
    assert_invariants(&v, &Policy::default(), None);
}

replica_update_tests! {
    prove_replica_update2 => prove_replica_update2_test,
    replica_update_full_path_success => replica_update_full_path_success_test,
    upgrade_and_miss_post => upgrade_and_miss_post_test,
    prove_replica_update_multi_dline => prove_replica_update_multi_dline_test,
    terminate_after_upgrade => terminate_after_upgrade_test,
    extend_after_upgrade => extend_after_upgrade_test,
    no_dispute_after_upgrade => nodispute_after_upgrade_test,
    upgrade_bad_post_dispute => upgrade_bad_post_dispute_test,
    bad_post_upgrade_dispute => bad_post_upgrade_dispute_test,
    // ---- Failure cases ----
    immutable_deadline_failure => immutable_deadline_failure_test,
    unhealthy_sector_failure => unhealthy_sector_failure_test,
    terminated_sector_failure => terminated_sector_failure_test,
    wrong_deadline_index_failure => wrong_deadline_index_failure_test,
    wrong_partition_index_failure => wrong_partition_index_failure_test,
    deal_included_in_multiple_sectors_failure => deal_included_in_multiple_sectors_failure_test,
}
