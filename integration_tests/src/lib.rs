use fvm_shared::{
    address::{Address, FIRST_NON_SINGLETON_ADDR},
    econ::TokenAmount,
    sector::StoragePower,
    smooth::FilterEstimate,
    ActorID,
};

pub mod deals;
pub mod expects;
pub mod tests;
pub mod util;

// accounts for verifreg root signer and msig
pub const VERIFREG_ROOT_KEY: &[u8] = &[200; fvm_shared::address::BLS_PUB_LEN];
pub const TEST_VERIFREG_ROOT_SIGNER_ID: ActorID = FIRST_NON_SINGLETON_ADDR;
pub const TEST_VERIFREG_ROOT_SIGNER_ADDR: Address = Address::new_id(TEST_VERIFREG_ROOT_SIGNER_ID);
pub const TEST_VERIFREG_ROOT_ID: ActorID = FIRST_NON_SINGLETON_ADDR + 1;
pub const TEST_VERIFREG_ROOT_ADDR: Address = Address::new_id(TEST_VERIFREG_ROOT_ID);

// account actor seeding funds created by new_with_singletons
pub const FAUCET_ROOT_KEY: &[u8] = &[153; fvm_shared::address::BLS_PUB_LEN];
pub const TEST_FAUCET_ADDR: Address = Address::new_id(FIRST_NON_SINGLETON_ADDR + 2);
pub const FIRST_TEST_USER_ADDR: ActorID = FIRST_NON_SINGLETON_ADDR + 3;

// static values for predictable testing
pub const TEST_VM_RAND_ARRAY: [u8; 32] = [
    1u8, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21, 22, 23, 24, 25,
    26, 27, 28, 29, 30, 31, 32,
];
pub const TEST_VM_INVALID_POST: &str = "i_am_invalid_post";

pub struct MinerBalances {
    pub available_balance: TokenAmount,
    pub vesting_balance: TokenAmount,
    pub initial_pledge: TokenAmount,
    pub pre_commit_deposit: TokenAmount,
}

pub struct NetworkStats {
    pub total_raw_byte_power: StoragePower,
    pub total_bytes_committed: StoragePower,
    pub total_quality_adj_power: StoragePower,
    pub total_qa_bytes_committed: StoragePower,
    pub total_pledge_collateral: TokenAmount,
    pub this_epoch_raw_byte_power: StoragePower,
    pub this_epoch_quality_adj_power: StoragePower,
    pub this_epoch_pledge_collateral: TokenAmount,
    pub miner_count: i64,
    pub miner_above_min_power_count: i64,
    pub this_epoch_reward: TokenAmount,
    pub this_epoch_reward_smoothed: FilterEstimate,
    pub this_epoch_baseline_power: StoragePower,
    pub total_storage_power_reward: TokenAmount,
    pub total_client_locked_collateral: TokenAmount,
    pub total_provider_locked_collateral: TokenAmount,
    pub total_client_storage_fee: TokenAmount,
}
