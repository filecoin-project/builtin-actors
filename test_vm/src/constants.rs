use fil_actors_runtime::FIRST_NON_SINGLETON_ADDR;
use fvm_shared::{address::Address, ActorID};

// TODO: Deduplicate these constants which currently exist both here and in the integration_tests crate.
// https://github.com/filecoin-project/builtin-actors/issues/1348

// accounts for verifreg root signer and msig
pub const VERIFREG_ROOT_KEY: &[u8] = &[200; fvm_shared::address::BLS_PUB_LEN];
pub const TEST_VERIFREG_ROOT_SIGNER_ADDR: Address = Address::new_id(FIRST_NON_SINGLETON_ADDR);
pub const TEST_VERIFREG_ROOT_ADDR: Address = Address::new_id(FIRST_NON_SINGLETON_ADDR + 1);

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
