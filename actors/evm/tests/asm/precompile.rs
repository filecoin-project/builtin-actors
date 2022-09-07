use super::asm;

use evm::interpreter::U256;
use fil_actor_evm as evm;
use fil_actors_runtime::test_utils::*;
use fvm_ipld_encoding::RawBytes;

#[allow(dead_code)]
pub fn magic_precompile_contract() -> Vec<u8> {
    let init = r#"
"#;

    let body = r#"
    return
"#;

    asm::new_contract("magic-precompile", init, body).unwrap()
}

#[test]
fn test_precompile_hash() {
    let contract = magic_precompile_contract();

    let mut rt = MockRuntime::default();

    // invoke constructor
    rt.expect_validate_caller_any();

    let params =
        evm::ConstructorParams { bytecode: contract.into(), input_data: RawBytes::default() };

    let result = rt
        .call::<evm::EvmContractActor>(
            evm::Method::Constructor as u64,
            &RawBytes::serialize(params).unwrap(),
        )
        .unwrap();
    expect_empty(result);
    rt.verify();

    // invoke contract
    let contract_params = vec![0u8; 32];
    let params = evm::InvokeParams { input_data: RawBytes::from(contract_params) };

    rt.expect_validate_caller_any();
    let result = rt
        .call::<evm::EvmContractActor>(
            evm::Method::InvokeContract as u64,
            &RawBytes::serialize(params).unwrap(),
        )
        .unwrap();

    let expected =
        hex_literal::hex!("527c30564edf3cb6da32e55ac39c4e93b9d9dfffde64663638b2a0bc33fa50c4");
    assert_eq!(
        U256::from_big_endian(&result),
        U256::from(expected),
        "\n{}\n{}",
        hex::encode(&*result),
        hex::encode(expected)
    );
}
