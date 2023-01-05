use fil_actor_evm::{State, Tombstone};
use fil_actors_runtime::test_utils::*;
use fvm_ipld_encoding::RawBytes;
use fvm_shared::{address::Address, error::ExitCode, METHOD_SEND};

mod util;

#[test]
fn test_selfdestruct() {
    let bytecode = hex::decode(include_str!("contracts/selfdestruct.hex")).unwrap();

    let contract = Address::new_id(100);
    let beneficiary = Address::new_id(1001);

    let mut rt = util::init_construct_and_verify(bytecode, |rt| {
        rt.actor_code_cids.insert(contract, *EVM_ACTOR_CODE_ID);
        rt.set_origin(contract);
    });

    let solidity_params = hex::decode("35f46994").unwrap();
    rt.expect_validate_caller_any();
    rt.expect_send(
        beneficiary,
        METHOD_SEND,
        None,
        rt.get_balance(),
        RawBytes::default(),
        ExitCode::OK,
    );

    assert!(util::invoke_contract(&mut rt, &solidity_params).is_empty());
    let state: State = rt.get_state();
    assert_eq!(state.tombstone, Some(Tombstone { origin: 100, nonce: 0 }));
    rt.verify()
}

#[test]
fn test_selfdestruct_missing() {
    let bytecode = hex::decode(include_str!("contracts/selfdestruct.hex")).unwrap();

    let contract = Address::new_id(100);
    let beneficiary = Address::new_id(1001);

    let mut rt = util::init_construct_and_verify(bytecode, |rt| {
        rt.actor_code_cids.insert(contract, *EVM_ACTOR_CODE_ID);
        rt.set_origin(contract);
    });

    let solidity_params = hex::decode("35f46994").unwrap();
    rt.expect_validate_caller_any();
    rt.expect_send(
        beneficiary,
        METHOD_SEND,
        None,
        rt.get_balance(),
        RawBytes::default(),
        ExitCode::SYS_INVALID_RECEIVER,
    );

    // It still works even if the beneficiary doesn't exist.
    assert!(util::invoke_contract(&mut rt, &solidity_params).is_empty());
    let state: State = rt.get_state();
    assert_eq!(state.tombstone, Some(Tombstone { origin: 100, nonce: 0 }));
    rt.verify();
}
