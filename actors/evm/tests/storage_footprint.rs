use std::sync::Arc;

use ethers::contract::Lazy;
use ethers::prelude::abigen;
use ethers::providers::{MockProvider, Provider};
use fvm_shared::address::Address;

mod env;

use env::TestEnv;

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

// The owner doesn't matter in these tests, so just using the same value that the other tests use, for everything.
const OWNER: Address = Address::new_id(100);

static CONTRACT: Lazy<StorageFootprint<Provider<MockProvider>>> = Lazy::new(|| {
    // The owner of the contract is expected to be the 160 bit hash used on Ethereum.
    // We're not going to use it during the tests.
    let owner_hex = format!("{:0>40}", hex::encode(OWNER.payload_bytes()));
    let address = owner_hex.parse::<ethers::core::types::Address>().unwrap();
    // A dummy client that we don't intend to use to call the contract or send transactions.
    let (client, _mock) = Provider::mocked();
    StorageFootprint::new(address, Arc::new(client))
});

/// Create a fresh test environment.
fn new_footprint_env() -> TestEnv {
    let mut env = TestEnv::new(OWNER);
    env.deploy(include_str!("contracts/StorageFootprint.hex"));
    env
}

#[test]
fn basic() {
    let mut env = new_footprint_env();
    let sum = env.call(|| CONTRACT.array_1_sum(0, 0));
    assert_eq!(sum, 0)
}

#[test]
fn measure_array1_push() {
    // Number of items to push at the end of the array at a time.
    for n in vec![1, 100] {
        let mut env = new_footprint_env();
        // Number of pushes to do on the same array, to see how its size affects the cost.
        for i in 1..=100 {
            env.runtime().store.clear_stats();
            env.call(|| CONTRACT.array_1_push(n));
            let stats = env.runtime().store.stats();
            eprintln!("n={}, i={}, {:?}", n, i, stats);
        }
    }
}
