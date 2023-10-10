mod authenticate_message_test;
pub use authenticate_message_test::*;
mod batch_onboarding;
pub use batch_onboarding::*;
mod batch_onboarding_deals_test;
pub use batch_onboarding_deals_test::*;
mod change_beneficiary_test;
pub use change_beneficiary_test::*;
mod change_owner_test;
pub use change_owner_test::*;
mod commit_post_test;
pub use commit_post_test::*;
mod datacap_tests;
pub use datacap_tests::*;
mod evm_test;
pub use evm_test::*;
mod extend_sectors_test;
pub use extend_sectors_test::*;
mod market_miner_withdrawal_test;
pub use market_miner_withdrawal_test::*;
mod multisig_test;
pub use multisig_test::*;
mod init_test;
pub use init_test::*;
mod power_scenario_tests;
pub use power_scenario_tests::*;
mod publish_deals_test;
pub use publish_deals_test::*;
mod replica_update_test;
pub use replica_update_test::*;
mod terminate_test;
pub use terminate_test::*;
mod verified_claim_test;
pub use verified_claim_test::*;
mod verifreg_remove_datacap_test;
pub use verifreg_remove_datacap_test::*;
mod withdraw_balance_test;
use vm_api::VM;
pub use withdraw_balance_test::*;
mod move_partitions_test;
pub use move_partitions_test::*;

use lazy_static::lazy_static;
use std::collections::HashMap;
use std::sync::Mutex;

type TestFn = fn(&dyn VM) -> ();

lazy_static! {
    pub static ref TEST_REGISTRY: Mutex<HashMap<String, TestFn>> = Mutex::new(HashMap::new());
}
