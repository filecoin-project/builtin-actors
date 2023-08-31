use fil_actors_integration_tests::tests::{
    evm_call_test, evm_create_test, evm_delegatecall_test, evm_empty_initcode_test,
    evm_eth_create_external_test, evm_init_revert_data_test, evm_staticcall_delegatecall_test,
    evm_staticcall_test,
};
use fvm_ipld_blockstore::MemoryBlockstore;
use test_vm::TestVM;

#[test]
fn evm_call() {
    let store = MemoryBlockstore::new();
    let v = TestVM::<MemoryBlockstore>::new_with_singletons(&store);
    evm_call_test(&v);
}

#[test]
fn evm_create() {
    let store = MemoryBlockstore::new();
    let v = TestVM::<MemoryBlockstore>::new_with_singletons(&store);
    evm_create_test(&v);
}

#[test]
fn evm_eth_create_external() {
    let store = MemoryBlockstore::new();
    let v = TestVM::<MemoryBlockstore>::new_with_singletons(&store);
    evm_eth_create_external_test(&v);
}

#[test]
fn evm_empty_initcode() {
    let store = MemoryBlockstore::new();
    let v = TestVM::<MemoryBlockstore>::new_with_singletons(&store);
    evm_empty_initcode_test(&v);
}
#[test]
fn evm_staticcall() {
    let store = MemoryBlockstore::new();
    let v = TestVM::<MemoryBlockstore>::new_with_singletons(&store);
    evm_staticcall_test(&v);
}

#[test]
fn evm_delegatecall() {
    let store = MemoryBlockstore::new();
    let v = TestVM::<MemoryBlockstore>::new_with_singletons(&store);
    evm_delegatecall_test(&v);
}

#[test]
fn evm_staticcall_delegatecall() {
    let store = MemoryBlockstore::new();
    let v = TestVM::<MemoryBlockstore>::new_with_singletons(&store);
    evm_staticcall_delegatecall_test(&v);
}

#[test]
fn evm_init_revert_data() {
    let store = MemoryBlockstore::new();
    let v = TestVM::<MemoryBlockstore>::new_with_singletons(&store);
    evm_init_revert_data_test(&v);
}
