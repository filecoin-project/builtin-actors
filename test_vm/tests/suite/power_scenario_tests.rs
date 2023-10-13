use fil_actors_integration_tests::tests::{cron_tick_test, power_create_miner_test};
use fil_actors_runtime::test_blockstores::TrackingMemBlockstore;
use test_vm::TestVM;

#[test]
fn power_create_miner() {
    let store = TrackingMemBlockstore::new();
    let v = TestVM::new_with_singletons(&store);

    power_create_miner_test(&v);
}

#[test]
fn cron_tick() {
    let store = TrackingMemBlockstore::new();
    let v = TestVM::new_with_singletons(&store);

    cron_tick_test(&v);
}
