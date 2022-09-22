use std::fs::File;
use std::io::Write;
use std::sync::Arc;

use ethers::contract::Lazy;
use ethers::prelude::abigen;
use ethers::providers::{MockProvider, Provider};
use fvm_shared::address::Address;

mod env;

use env::{BlockstoreStats, TestEnv};
use serde_json::json;

// Generate a statically typed interface for the contract.
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

struct Measurements {
    scenario: String,
    values: Vec<serde_json::Value>,
}

impl Measurements {
    pub fn new(scenario: String) -> Self {
        Self { scenario, values: Vec::new() }
    }

    pub fn record(&mut self, series: u8, i: u32, stats: BlockstoreStats) {
        // Not merging `i` into `stats` in JSON so the iteration appear in the left.
        let value = json!({
            "i": i,
            "series": series,
            "stats": stats
        });
        self.values.push(value);
    }

    pub fn export(self) -> Result<(), std::io::Error> {
        let dir = std::env::var("CARGO_MANIFEST_DIR").unwrap();
        let path = format!("{}/tests/measurements/{}.jsonline", dir, self.scenario);
        let mut output = File::create(path)?;
        for value in self.values {
            writeln!(output, "{}", value)?;
        }
        Ok(())
    }
}

#[test]
fn basic() {
    let mut env = new_footprint_env();
    let sum = env.call(|| CONTRACT.array_1_sum(0, 0));
    assert_eq!(sum, 0)
}

/// Number of iterations to do in a scenario, ie. the number of observations along the X axis.
const NUM_ITER: u32 = 100;

/// Measure the cost of pushing items into dynamic arrays. Run multiple scenarios
/// with different number of items pushed in one call. First do a number of iterations
/// with `array1`, then with `array2` to see if the former affects the latter.
#[test]
fn measure_array_push() {
    // Number of items to push at the end of the array at a time.
    for n in [1, 100] {
        let mut env = new_footprint_env();
        let mut mts = Measurements::new(format!("array_push_n{}", n));
        for i in 0..NUM_ITER {
            env.call(|| CONTRACT.array_1_push(n));
            mts.record(1, i, env.runtime().store.take_stats());
        }
        for i in 0..NUM_ITER {
            env.call(|| CONTRACT.array_2_push(n));
            mts.record(2, i, env.runtime().store.take_stats());
        }
        mts.export().unwrap()
    }
}

/// Measure the cost of adding items to a mapping. Run multiple scenarios with different
/// number of items added at the same time. Add to `mapping1` first, then `mapping2`.
#[test]
fn measure_mapping_add() {
    // Number of items to add to the mapping at a time
    for n in [1, 100] {
        let mut env = new_footprint_env();
        let mut mts = Measurements::new(format!("mapping_add_n{}", n));
        // In this case we always add new keys, never overwrite existing ones, to compare to
        // the scenario where we were pushing to the end of arrays.
        for i in 0..NUM_ITER {
            env.call(|| CONTRACT.mapping_1_set(i * n, n, i));
            mts.record(1, i, env.runtime().store.take_stats());
        }
        for i in 0..NUM_ITER {
            env.call(|| CONTRACT.mapping_2_set(i * n, n, i));
            mts.record(2, i, env.runtime().store.take_stats());
        }
        mts.export().unwrap()
    }
}

/// Fill the array with 10,000 items, then read varying number of consecutive entries from it.
#[test]
fn measure_array_read() {
    let mut env = new_footprint_env();
    env.call(|| CONTRACT.array_1_push(10000));
    env.runtime().store.clear_stats();

    // Number of items to access from the array at a time.
    for n in [1, 100] {
        let mut mts = Measurements::new(format!("array_read_n{}", n));
        for i in 0..NUM_ITER {
            env.call(|| CONTRACT.array_1_sum(i * n, n));
            mts.record(1, i, env.runtime().store.take_stats());
        }
        mts.export().unwrap()
    }
}
