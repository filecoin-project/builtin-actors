use evm::interpreter::address::EthAddress;
use fil_actor_evm as evm;
use fil_actors_runtime::{runtime::builtins::Type, test_utils::*, INIT_ACTOR_ADDR};
use fvm_ipld_encoding::{BytesDe, BytesSer, RawBytes};
use fvm_shared::address::Address;

#[allow(dead_code)]
pub fn construct_and_verify(initcode: Vec<u8>) -> MockRuntime {
    init_construct_and_verify(initcode, |_| {})
}

pub fn init_construct_and_verify<F: FnOnce(&mut MockRuntime)>(
    initcode: Vec<u8>,
    initrt: F,
) -> MockRuntime {
    let mut rt = MockRuntime::default();

    // construct EVM actor
    rt.set_caller(*INIT_ACTOR_CODE_ID, INIT_ACTOR_ADDR);
    rt.expect_validate_caller_type(vec![Type::Init]);
    initrt(&mut rt);

    // first actor created is 0
    rt.add_delegated_address(
        Address::new_id(0),
        Address::new_delegated(10, &hex_literal::hex!("FEEDFACECAFEBEEF000000000000000000000000"))
            .unwrap(),
    );

    let params = evm::ConstructorParams {
        creator: EthAddress::from_id(fil_actors_runtime::EAM_ACTOR_ADDR.id().unwrap()),
        initcode: initcode.into(),
    };

    assert!(rt
        .call::<evm::EvmContractActor>(
            evm::Method::Constructor as u64,
            &RawBytes::serialize(params).unwrap(),
        )
        .unwrap()
        .is_empty());
    rt.verify();

    rt
}

#[allow(dead_code)]
pub fn invoke_contract(rt: &mut MockRuntime, input_data: &[u8]) -> Vec<u8> {
    rt.expect_validate_caller_any();
    let BytesDe(res) = rt
        .call::<evm::EvmContractActor>(
            evm::Method::InvokeContract as u64,
            &RawBytes::serialize(BytesSer(input_data)).unwrap(),
        )
        .unwrap()
        .deserialize()
        .unwrap();
    res
}
