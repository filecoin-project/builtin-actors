use ethers::{
    abi::Detokenize,
    prelude::{builders::ContractCall, decode_function_data},
    providers::{MockProvider, Provider},
};
use fil_actor_evm as evm;
use fil_actors_evm_shared::address::EthAddress;
use fil_actors_runtime::{
    test_utils::{MockRuntime, EVM_ACTOR_CODE_ID, INIT_ACTOR_CODE_ID},
    INIT_ACTOR_ADDR,
};
use fvm_ipld_blockstore::tracking::{BSStats, TrackingBlockstore};
use fvm_ipld_blockstore::MemoryBlockstore;
use fvm_ipld_encoding::ipld_block::IpldBlock;
use fvm_ipld_encoding::{BytesDe, BytesSer};
use fvm_shared::address::Address;

/// Alias for a call we will never send to the blockchain.
pub type TestContractCall<R> = ContractCall<Provider<MockProvider>, R>;

pub struct TestEnv {
    evm_address: Address,
    pub runtime: MockRuntime<TrackingBlockstore<MemoryBlockstore>>,
}

impl TestEnv {
    pub fn take_store_stats(&mut self) -> BSStats {
        self.runtime.store.stats.take()
    }

    pub fn clear_store_stats(&mut self) {
        self.take_store_stats();
    }

    /// Create a new test environment where the EVM actor code is already
    /// loaded under an actor address.
    pub fn new(evm_address: Address) -> Self {
        let runtime = MockRuntime::new(TrackingBlockstore::new(MemoryBlockstore::new()));

        runtime.actor_code_cids.borrow_mut().insert(evm_address, *EVM_ACTOR_CODE_ID);

        Self { evm_address, runtime }
    }

    /// Deploy a contract into the EVM actor.
    pub fn deploy(&mut self, contract_hex: &str) {
        let params = evm::ConstructorParams {
            creator: EthAddress::from_id(fil_actors_runtime::EAM_ACTOR_ADDR.id().unwrap()),
            initcode: hex::decode(contract_hex).unwrap().into(),
        };
        // invoke constructor
        self.runtime.expect_validate_caller_addr(vec![INIT_ACTOR_ADDR]);
        self.runtime.set_caller(*INIT_ACTOR_CODE_ID, INIT_ACTOR_ADDR);

        self.runtime.set_origin(self.evm_address);
        // first actor created is 0
        self.runtime.set_delegated_address(
            0,
            Address::new_delegated(
                10,
                &hex_literal::hex!("FEEDFACECAFEBEEF000000000000000000000000"),
            )
            .unwrap(),
        );

        assert!(self
            .runtime
            .call::<evm::EvmContractActor>(
                evm::Method::Constructor as u64,
                IpldBlock::serialize_cbor(&params).unwrap(),
            )
            .unwrap()
            .is_none());

        self.runtime.verify();
    }

    /// Take a function that calls an ABI method to return a `ContractCall`.
    /// Then, instead of calling the contract on-chain, run it through our
    /// EVM interpreter in the test runtime. Finally parse the results.
    pub fn call<R: Detokenize>(&mut self, call: TestContractCall<R>) -> R {
        let input = call.calldata().expect("Should have calldata.");
        let input =
            IpldBlock::serialize_cbor(&BytesSer(&input)).expect("failed to serialize input data");
        self.runtime.expect_validate_caller_any();

        let BytesDe(result) = self
            .runtime
            .call::<evm::EvmContractActor>(evm::Method::InvokeContract as u64, input)
            .unwrap()
            .unwrap()
            .deserialize()
            .unwrap();

        decode_function_data(&call.function, result, false).unwrap()
    }
}
