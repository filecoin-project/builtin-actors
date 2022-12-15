use cid::Cid;
use evm::interpreter::{address::EthAddress, StatusCode};
use fil_actor_evm as evm;
use fil_actors_runtime::{test_utils::*, ActorError, EAM_ACTOR_ID, INIT_ACTOR_ADDR};
use fvm_ipld_encoding::{BytesDe, BytesSer, RawBytes};
use fvm_shared::{address::Address, IDENTITY_HASH, IPLD_RAW};
use lazy_static::lazy_static;

#[allow(dead_code)]
pub fn construct_and_verify(initcode: Vec<u8>) -> MockRuntime {
    init_construct_and_verify(initcode, |_| {})
}

pub const CONTRACT_ADDRESS: [u8; 20] =
    hex_literal::hex!("FEEDFACECAFEBEEF000000000000000000000000");

pub fn init_construct_and_verify<F: FnOnce(&mut MockRuntime)>(
    initcode: Vec<u8>,
    initrt: F,
) -> MockRuntime {
    let mut rt = MockRuntime::default();

    // construct EVM actor
    rt.set_caller(*INIT_ACTOR_CODE_ID, INIT_ACTOR_ADDR);
    rt.expect_validate_caller_addr(vec![INIT_ACTOR_ADDR]);
    initrt(&mut rt);

    // first actor created is 0
    rt.add_delegated_address(
        Address::new_id(0),
        Address::new_delegated(EAM_ACTOR_ID, &CONTRACT_ADDRESS).unwrap(),
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

#[allow(dead_code)]
pub fn invoke_contract_expect_abort(rt: &mut MockRuntime, input_data: &[u8], expect: StatusCode) {
    rt.expect_validate_caller_any();
    let err = rt
        .call::<evm::EvmContractActor>(
            evm::Method::InvokeContract as u64,
            &RawBytes::serialize(BytesSer(input_data)).unwrap(),
        )
        .expect_err(&format!("expected contract to fail with {}", expect));
    rt.verify();
    // REMOVEME so this is jank... (just copies err creation from execute in lib.rs)
    assert_eq!(err, ActorError::unspecified(format!("EVM execution error: {expect:?}")))
}

#[allow(dead_code)]
pub fn dispatch_num_word(method_num: u8) -> [u8; 32] {
    let mut word = [0u8; 32];
    word[3] = method_num;
    word
}

lazy_static! {
    pub static ref DUMMY_ACTOR_CODE_ID: Cid =
        Cid::new_v1(IPLD_RAW, Multihash::wrap(IDENTITY_HASH, b"foobarboxy").unwrap());
}
