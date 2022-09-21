use std::cell::Cell;

use anyhow::Result;
use cid::Cid;
use ethers::{
    abi::Detokenize,
    prelude::{builders::ContractCall, decode_function_data},
    providers::{MockProvider, Provider},
};
use fil_actor_evm as evm;
use fil_actors_runtime::test_utils::{expect_empty, MockRuntime, EVM_ACTOR_CODE_ID};
use fvm_ipld_blockstore::{Blockstore, MemoryBlockstore};
use fvm_ipld_encoding::RawBytes;
use fvm_shared::address::Address;

#[derive(Copy, Clone, Default, Debug)]
pub struct BlockstoreStats {
    /// Number of calls to `get`.
    pub get_count: usize,
    /// Number of calls to `put`.
    pub put_count: usize,
    /// Total bytes read by `get`.
    pub get_bytes: usize,
    /// Total bytes written by `put`.
    pub put_bytes: usize,
}

#[derive(Default)]
pub struct TrackingBlockstore {
    store: MemoryBlockstore,
    stats: Cell<BlockstoreStats>,
}

impl TrackingBlockstore {
    /// Current snapshot of the stats.
    pub fn stats(&self) -> BlockstoreStats {
        self.stats.get()
    }

    /// Reset stats to zero between.
    pub fn clear_stats(&self) {
        self.stats.set(BlockstoreStats::default())
    }
}

impl Blockstore for TrackingBlockstore {
    fn has(&self, k: &Cid) -> Result<bool> {
        self.store.has(k)
    }

    fn get(&self, k: &Cid) -> Result<Option<Vec<u8>>> {
        let mut stats = self.stats.get();
        stats.get_count += 1;
        let block = self.store.get(k)?;
        if let Some(block) = &block {
            stats.get_bytes += block.len();
        }
        self.stats.set(stats);
        Ok(block)
    }

    fn put_keyed(&self, k: &Cid, block: &[u8]) -> Result<()> {
        let mut stats = self.stats.get();
        stats.put_count += 1;
        stats.put_bytes += block.len();
        self.stats.set(stats);
        self.store.put_keyed(k, block)
    }
}

pub struct TestEnv {
    evm_address: Address,
    runtime: MockRuntime<TrackingBlockstore>,
}

impl TestEnv {
    pub fn runtime(&self) -> &MockRuntime<TrackingBlockstore> {
        &self.runtime
    }

    /// Create a new test environment where the EVM actor code is already
    /// loaded under an actor address.
    pub fn new(evm_address: Address) -> Self {
        let mut runtime = MockRuntime::new(TrackingBlockstore::default());

        runtime.actor_code_cids.insert(evm_address, *EVM_ACTOR_CODE_ID);

        Self { evm_address, runtime }
    }

    /// Deploy a contract into the EVM actor.
    pub fn deploy(&mut self, contract_hex: &str) {
        let params = evm::ConstructorParams { bytecode: hex::decode(contract_hex).unwrap().into() };
        // invoke constructor
        self.runtime.expect_validate_caller_any();
        self.runtime.set_origin(self.evm_address);

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

    /// Take a function that calls an ABI method to return a `ContractCall`.
    /// Then, instead of calling the contract on-chain, run it through our
    /// EVM interpreter in the test runtime. Finally parse the results.
    pub fn call<F, R>(&mut self, f: F) -> R
    where
        F: FnOnce() -> ContractCall<Provider<MockProvider>, R>,
        R: Detokenize,
    {
        let call = f();
        let input = call.calldata().expect("Should have calldata.");
        let input = RawBytes::from(input.to_vec());
        self.runtime.expect_validate_caller_any();

        let result = self
            .runtime
            .call::<evm::EvmContractActor>(evm::Method::InvokeContract as u64, &input)
            .unwrap();

        decode_function_data(&call.function, result.bytes(), false).unwrap()
    }
}
