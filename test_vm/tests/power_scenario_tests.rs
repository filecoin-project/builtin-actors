use fil_actors_integration_tests::tests::{cron_tick_test, power_create_miner_test};
use test_vm::new_test_vm;

#[test]
fn power_create_miner() {
    let v = new_test_vm();

    power_create_miner_test(&*v);
}

#[test]
fn cron_tick() {
    let v = new_test_vm();

    cron_tick_test(&*v);
}
