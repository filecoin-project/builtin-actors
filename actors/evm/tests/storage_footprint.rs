use std::sync::Arc;

use ethers::prelude::abigen;
use ethers::providers::{MockProvider, Provider};
use fil_actor_evm as evm;
use fil_actors_runtime::test_utils::{expect_empty, MockRuntime, EVM_ACTOR_CODE_ID};
use fvm_ipld_encoding::RawBytes;
use fvm_shared::address::Address;

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

struct TestEnv {
    runtime: MockRuntime,
}

impl TestEnv {
    /// Create a new test environment where the EVM actor code is already
    /// loaded under the global owner.
    pub fn new() -> Self {
        let mut runtime = MockRuntime::default();
        runtime.actor_code_cids.insert(OWNER, *EVM_ACTOR_CODE_ID);
        Self { runtime }
    }

    /// Deploy a contract into the EVM actor.
    pub fn deploy(&mut self, contract_hex: &str) {
        let params = evm::ConstructorParams { bytecode: hex::decode(contract_hex).unwrap().into() };
        // invoke constructor
        self.runtime.expect_validate_caller_any();
        self.runtime.set_origin(OWNER);

        let result = self
            .runtime
            .call::<evm::EvmContractActor>(
                evm::Method::Constructor as u64,
                &RawBytes::serialize(params).unwrap(),
            )
            .unwrap();

        expect_empty(result);

        self.runtime.verify();
    }
}

impl Default for TestEnv {
    fn default() -> Self {
        Self::new()
    }
}

#[test]
fn basic() {
    let _ = StorageFootprint::default();
    let mut env = TestEnv::default();
    env.deploy(include_str!("contracts/StorageFootprint.hex"))
}
