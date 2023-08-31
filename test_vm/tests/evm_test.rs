use fil_actors_integration_tests::tests::{
    evm_call_test, evm_create_test, evm_delegatecall_test, evm_empty_initcode_test,
    evm_eth_create_external_test, evm_init_revert_data_test, evm_staticcall_delegatecall_test,
    evm_staticcall_test,
};
use test_vm::new_test_vm;

#[test]
fn evm_call() {
    let v = new_test_vm();
    evm_call_test(&*v);
}

#[test]
fn evm_create() {
    let v = new_test_vm();
    evm_create_test(&*v);
}

#[test]
fn evm_eth_create_external() {
    let v = new_test_vm();
    evm_eth_create_external_test(&*v);
}

#[test]
fn evm_empty_initcode() {
    let v = new_test_vm();
    evm_empty_initcode_test(&*v);
}
#[test]
fn evm_staticcall() {
    let v = new_test_vm();
    evm_staticcall_test(&*v);
}

#[test]
fn evm_delegatecall() {
    let v = new_test_vm();
    evm_delegatecall_test(&*v);
}

#[test]
fn evm_staticcall_delegatecall() {
    let v = new_test_vm();
    evm_staticcall_delegatecall_test(&*v);
}

#[test]
fn evm_init_revert_data() {
    let v = new_test_vm();
    evm_init_revert_data_test(&*v);
}
