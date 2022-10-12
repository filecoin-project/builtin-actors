use evm::interpreter::address::EthAddress;
use fil_actor_evm as evm;
use fil_actors_runtime::test_utils::*;
use fvm_ipld_encoding::RawBytes;

#[allow(dead_code)]
pub fn construct_and_verify(initcode: Vec<u8>) -> MockRuntime {
    init_construct_and_verify(initcode, |_| {})
}

pub fn init_construct_and_verify<F: FnOnce(&mut MockRuntime)>(
    initcode: Vec<u8>,
    initrt: F,
) -> MockRuntime {
    let mut rt = MockRuntime::default();

    // invoke constructor
    rt.expect_validate_caller_any();
    initrt(&mut rt);

    let params = evm::ConstructorParams { creator: EthAddress::from_id(fil_actors_runtime::EAM_ACTOR_ADDR.id().unwrap()), initcode: initcode.into() };

    let result = rt
        .call::<evm::EvmContractActor>(
            evm::Method::Constructor as u64,
            &RawBytes::serialize(params).unwrap(),
        )
        .unwrap();
    expect_empty(result);
    rt.verify();

    rt
}

#[allow(dead_code)]
pub fn invoke_contract(rt: &mut MockRuntime, input_data: RawBytes) -> RawBytes {
    rt.expect_validate_caller_any();
    rt.call::<evm::EvmContractActor>(evm::Method::InvokeContract as u64, &input_data).unwrap()
}
