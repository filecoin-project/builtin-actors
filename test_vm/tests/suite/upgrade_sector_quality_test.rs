use fil_actors_integration_tests::tests::upgrade_sector_quality_upgrades_legacy_sector_test;
use fil_actors_runtime::test_blockstores::MemoryBlockstore;
use test_vm::TestVM;

#[test]
fn upgrade_sector_quality_upgrades_legacy_sector() {
    let store = MemoryBlockstore::new();
    let v = TestVM::new_with_singletons(store);
    upgrade_sector_quality_upgrades_legacy_sector_test(&v);
}
