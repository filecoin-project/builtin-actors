use fil_actors_runtime::test_utils::*;
use fvm_ipld_encoding::RawBytes;
use fvm_shared::address::Address;

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
    let input_data = RawBytes::from(solidity_params);
    rt.expect_validate_caller_any();
    rt.expect_delete_actor(beneficiary);

    let result = util::invoke_contract(&mut rt, input_data);
    expect_empty(result);
    rt.verify();
}
