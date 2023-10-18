mod authenticate_message_test;
mod batch_onboarding;
mod batch_onboarding_deals_test;
mod change_beneficiary_test;
mod change_owner_test;
mod commit_post_test;
mod datacap_tests;
mod evm_test;
mod extend_sectors_test;
mod init_test;
mod market_miner_withdrawal_test;
mod move_partitions_test;
mod multisig_test;
mod power_scenario_tests;
mod publish_deals_test;
mod replica_update_test;
mod terminate_test;
mod test_vm_test;
mod verified_claim_test;
mod verifreg_remove_datacap_test;
mod withdraw_balance_test;

use fil_actors_integration_tests::tests::TEST_REGISTRY;
use fil_actors_runtime::test_blockstores::MemoryBlockstore;
use test_vm::TestVM;

#[test]
pub fn run_all_tests() {
    for test in TEST_REGISTRY.lock().unwrap().iter() {
        println!("Running test: {}", test.0);
        let store = MemoryBlockstore::new();
        let v = TestVM::<MemoryBlockstore>::new_with_singletons(&store);
        test.1(&v);
    }
}
