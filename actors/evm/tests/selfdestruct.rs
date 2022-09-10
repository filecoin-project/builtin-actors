use fil_actor_evm as evm;
use fil_actors_runtime::test_utils::*;
use fvm_ipld_encoding::RawBytes;
use fvm_shared::address::Address;

#[test]
fn test_selfdestruct() {
    let mut rt = MockRuntime::default();

    let contract = Address::new_id(100);
    let beneficiary = Address::new_id(1001);

    let params = evm::ConstructorParams {
        bytecode: hex::decode(include_str!("selfdestruct.hex")).unwrap().into(),
        input_data: RawBytes::default(),
    };

    // invoke constructor
    rt.actor_code_cids.insert(contract, *EVM_ACTOR_CODE_ID);
    rt.expect_validate_caller_any();
    rt.set_origin(contract);

    let result = rt
        .call::<evm::EvmContractActor>(
            evm::Method::Constructor as u64,
            &RawBytes::serialize(params).unwrap(),
        )
        .unwrap();
    expect_empty(result);
    rt.verify();

    let solidity_params = hex::decode("35f46994").unwrap();
    let input_data = RawBytes::from(solidity_params);
    rt.expect_validate_caller_any();
    rt.expect_delete_actor(beneficiary);

    rt.call::<evm::EvmContractActor>(evm::Method::InvokeContract as u64, &input_data).unwrap();
    rt.verify();
}
