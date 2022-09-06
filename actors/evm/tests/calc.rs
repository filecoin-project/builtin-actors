mod asm;

use evm::interpreter::U256;
use fil_actor_evm as evm;
use fil_actors_runtime::test_utils::*;
use fvm_ipld_encoding::RawBytes;

#[allow(dead_code)]
pub fn magic_calc_contract() -> Vec<u8> {
    let init = r#"
push1 0x42  # magic value
push1 0x00  # key of magic value
sstore
"#;
    let body = r#"
# method dispatch:
# - 0x00000000 -> magic value
# - 0x00000001 -> ADD arg, magic value
# - 0x00000002 -> MUL arg, magic value

push1 0x00
calldataload
push1 0xe0   # 28 byte shift == 224 bits
shr

# 0x00 -> jmp get_magic
dup1
iszero
%push(get_magic)
jumpi

# 0x01 -> jmp add_magic
dup1
push1 0x01
eq
%push(add_magic)
jumpi

# 0x02 -> jmp mul_magic
dup1
push1 0x02
eq
%push(mul_magic)
jumpi

# unknown method, barf returning nothing
push1 0x00
dup1
revert

#### method implementation
get_magic:
jumpdest
push1 0x20 # length of return data
push1 0x00 # key of magic
sload
push1 0x00 # return memory offset
mstore
push1 0x00
return

add_magic:
jumpdest
push1 0x20   # length of return data
push1 0x04
calldataload # arg1
push1 0x00   # key of magic
sload
add
push1 0x00   # return memory offset
mstore
push1 0x00
return

mul_magic:
jumpdest
push1 0x20   # length of return dataa
push1 0x04
calldataload # arg1
push1 0x00   # key of magic
sload
mul
push1 0x00   # return memory offset
mstore
push1 0x00
return

"#;

    asm::new_contract(&"magic-calc", &init, &body).unwrap()
}

#[test]
fn test_magic_calc() {
    let contract = magic_calc_contract();

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

    // invoke contract -- get_magic
    let contract_params = vec![0u8; 32];
    let params = evm::InvokeParams { input_data: RawBytes::from(contract_params) };

    rt.expect_validate_caller_any();
    let result = rt
        .call::<evm::EvmContractActor>(
            evm::Method::InvokeContract as u64,
            &RawBytes::serialize(params).unwrap(),
        )
        .unwrap();

    assert_eq!(U256::from_big_endian(&result), U256::from(0x42));

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
