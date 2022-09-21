use std::sync::Arc;

use ethers::prelude::abigen;
use ethers::providers::{MockProvider, Provider};
use fvm_shared::address::Address;

mod env;

use env::TestEnv;

// The owner doesn't matter in these tests, so just using the same value that the other tests use, for everything.
const OWNER: Address = Address::new_id(100);

// Generate a statically types interface for the contract.
abigen!(StorageFootprint, "./tests/contracts/StorageFootprint.abi");

// Alternatively we can generate the ABI code as follows:
// ```
//     ethers::prelude::Abigen::new("StorageFootprint", "./tests/contracts/StorageFootprint.abi")
//         .unwrap()
//         .generate()
//         .unwrap()
//         .write_to_file("./tests/storage_footprint_abi.rs")
//         .unwrap();
// ```

/// Build a default StorageFootprint that we can use in tests.
impl Default for StorageFootprint<Provider<MockProvider>> {
    fn default() -> Self {
        // The owner of the contract is expected to be the 160 bit hash used on Ethereum.
        // We're not going to use it during the tests.
        let owner_hex = format!("{:0>40}", hex::encode(OWNER.payload_bytes()));
        let address = owner_hex.parse::<ethers::core::types::Address>().unwrap();
        // A dummy client that we don't intend to use to call the contract or send transactions.
        let (client, _mock) = Provider::mocked();
        Self::new(address, Arc::new(client))
    }
}

#[test]
fn basic() {
    let mut env = TestEnv::new(OWNER);
    env.deploy(include_str!("contracts/StorageFootprint.hex"));
    let contract = StorageFootprint::default();
    let sum = env.call(|| contract.array_1_sum(0, 0));
    assert_eq!(sum, 0)
}
