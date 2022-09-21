use std::sync::Arc;

use ethers::prelude::abigen;
use ethers::providers::{MockProvider, Provider};
use fvm_shared::address::Address;

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
fn abi_test() {
    let _ = StorageFootprint::default();
}
