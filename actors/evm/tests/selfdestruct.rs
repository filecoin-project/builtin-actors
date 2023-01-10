use fil_actors_runtime::{test_utils::*, BURNT_FUNDS_ACTOR_ADDR};
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
    rt.expect_delete_actor(BURNT_FUNDS_ACTOR_ADDR);

    assert!(util::invoke_contract(&mut rt, &solidity_params).is_empty());
    rt.verify();
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
    rt.expect_delete_actor(BURNT_FUNDS_ACTOR_ADDR);

    assert!(util::invoke_contract(&mut rt, &solidity_params).is_empty());
    rt.verify();
}
