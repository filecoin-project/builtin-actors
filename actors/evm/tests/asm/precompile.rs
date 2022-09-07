use super::asm;

use evm::interpreter::U256;
use fil_actor_evm as evm;
use fil_actors_runtime::test_utils::*;
use fvm_ipld_encoding::RawBytes;

#[allow(dead_code)]
pub fn magic_precompile_contract() -> Vec<u8> {
    let init = r#"
push32 0xACE8597929092C14BD028EDE7B07727875788C7E130278B5AFED41940D965ABA
push1 0x00 # expected data
sstore

push16 0x666F6F206261722062617A20626F7879 # foo bar baz boxy
push2 0x0100 # offset of input data
mstore # store value at offset


"#;

    let body = r#"
%push(sha256_hash)
jump # call hash, output written to 0x0200

sha256_hash:
jumpdest
push1 0x20   # out size (32 bytes)
push2 0x0200 # out offset 
push1 0x10   # in size (16 bytes)
push2 0x0100 # in offset
push1 0x00 # _value
push1 0x02 # dst TODO is this pushed to state properly? look at compiled contract calling precompile
push1 0x00 # _gas
call
push1 0x20
push2 0x0200 
return
"#;

    asm::new_contract("magic-precompile", init, body).unwrap()
}

#[test]
fn test_magic_calc() {
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

        
    let expected = hex_literal::hex!("ACE8597929092C14BD028EDE7B07727875788C7E130278B5AFED41940D965ABA");
    assert_eq!(U256::from_big_endian(&result), U256::from(expected), "\n{}\n{}", hex::encode(&*result), hex::encode(expected));

    // invoke contract -- add_magic
    let mut contract_params = vec![0u8; 36];
    contract_params[3] = 0x01;
    contract_params[35] = 0x01;
    let params = evm::InvokeParams { input_data: RawBytes::from(contract_params) };

    rt.expect_validate_caller_any();
    let result = rt
        .call::<evm::EvmContractActor>(
            evm::Method::InvokeContract as u64,
            &RawBytes::serialize(params).unwrap(),
        )
        .unwrap();

    assert_eq!(U256::from_big_endian(&result), U256::from(0x43));

    // invoke contract -- mul_magic
    let mut contract_params = vec![0u8; 36];
    contract_params[3] = 0x02;
    contract_params[35] = 0x02;
    let params = evm::InvokeParams { input_data: RawBytes::from(contract_params) };

    rt.expect_validate_caller_any();
    let result = rt
        .call::<evm::EvmContractActor>(
            evm::Method::InvokeContract as u64,
            &RawBytes::serialize(params).unwrap(),
        )
        .unwrap();

    assert_eq!(U256::from_big_endian(&result), U256::from(0x84));
}
