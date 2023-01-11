mod asm;

use evm::interpreter::{address::EthAddress, U256};
use fil_actor_evm as evm;
use fil_actors_runtime::test_utils::{
    MockRuntime, ACCOUNT_ACTOR_CODE_ID, EAM_ACTOR_CODE_ID, EVM_ACTOR_CODE_ID, MINER_ACTOR_CODE_ID,
    MULTISIG_ACTOR_CODE_ID, PLACEHOLDER_ACTOR_CODE_ID,
};
use fvm_shared::address::Address as FILAddress;

mod util;
use util::id_to_vec;

#[allow(dead_code)]
pub fn magic_precompile_contract() -> Vec<u8> {
    let init = r#"
"#;

    let body = r#"
push16 0x666F6F206261722062617A20626F7879 # foo bar baz boxy
push2 0x0100 # offset of input data
mstore # store value at offset

%push(sha256_hash)
jump # call hash, output written to 0x0200

sha256_hash:
jumpdest
push1 0x20   # out size (32 bytes)
push2 0x0200 # out offset
push1 0x10   # in size (16 bytes)
push2 0x0110 # in offset
push1 0x00 # _value
push1 0x02 # dst (0x02 is keccak-256)
push1 0x00 # _gas
call
push1 0x20
push2 0x0200
return
"#;

    asm::new_contract("magic-precompile", init, body).unwrap()
}

#[test]
fn test_precompile_hash() {
    let contract = magic_precompile_contract();
    let mut rt = util::construct_and_verify(contract);

    // invoke contract
    let contract_params = vec![0u8; 32];

    rt.expect_gas_available(10_000_000_000u64);
    let result = util::invoke_contract(&mut rt, &contract_params);
    let expected =
        hex_literal::hex!("ace8597929092c14bd028ede7b07727875788c7e130278b5afed41940d965aba");
    assert_eq!(
        U256::from_big_endian(&result),
        U256::from(expected),
        "\n{}\n{}",
        hex::encode(&*result),
        hex::encode(expected)
    );
}

#[test]
fn test_native_actor_type() {
    let bytecode = {
        let init = "";
        let body = r#"
    
# get call payload size
calldatasize
# store payload to mem 0x00
push1 0x00
push1 0x00
calldatacopy

# out size
# out off
push1 0x20
push1 0xA0

# in size
# in off
calldatasize
push1 0x00

# value
push1 0x00

# dst (get_actor_type precompile)
push20 0xfe00000000000000000000000000000000000004

# gas
push1 0x00

call

# copy result to mem 0x00 (overwrites input data)
returndatasize
push1 0x00
push1 0x00
returndatacopy

# return
returndatasize
push1 0x00
return
"#;

        asm::new_contract("native_actor_type", init, body).unwrap()
    };

    use evm::interpreter::precompiles::NativeType;

    let mut rt = util::construct_and_verify(bytecode);

    // 0x88 is an EVM actor
    let evm_target = FILAddress::new_id(0x88);
    rt.set_address_actor_type(evm_target, *EVM_ACTOR_CODE_ID);

    // f0 31 is a system actor
    let system_target = FILAddress::new_id(10);
    rt.set_address_actor_type(system_target, *EAM_ACTOR_CODE_ID);

    // f0 101 is an account
    let account_target = FILAddress::new_id(101);
    rt.set_address_actor_type(account_target, *ACCOUNT_ACTOR_CODE_ID);

    // f0 102 is a placeholder
    let placeholder_target = FILAddress::new_id(102);
    rt.set_address_actor_type(placeholder_target, *PLACEHOLDER_ACTOR_CODE_ID);

    // f0 103 is a storage provider
    let miner_target = FILAddress::new_id(103);
    rt.set_address_actor_type(miner_target, *MINER_ACTOR_CODE_ID);

    // f0 104 is a multisig
    let other_target = FILAddress::new_id(104);
    rt.set_address_actor_type(other_target, *MULTISIG_ACTOR_CODE_ID);

    fn test_type(rt: &mut MockRuntime, id: FILAddress, expected: NativeType) {
        rt.expect_gas_available(10_000_000_000u64);
        let result = util::invoke_contract(rt, &id_to_vec(&id));
        rt.verify();
        assert_eq!(&U256::from(expected as u32).to_bytes(), result.as_slice());
        rt.reset();
    }

    test_type(&mut rt, evm_target, NativeType::EVMContract);
    test_type(&mut rt, system_target, NativeType::System);
    test_type(&mut rt, account_target, NativeType::Account);
    test_type(&mut rt, placeholder_target, NativeType::Placeholder);
    test_type(&mut rt, miner_target, NativeType::StorageProvider);
    test_type(&mut rt, other_target, NativeType::OtherTypes);
    test_type(&mut rt, FILAddress::new_id(10101), NativeType::NonExistent);

    // invalid format address
    rt.expect_gas_available(10_000_000_000u64);
    let result = util::invoke_contract(&mut rt, &[0xff; 64]);
    rt.verify();
    assert!(result.is_empty());
    rt.reset();
}

fn resolve_address_contract() -> Vec<u8> {
    let init = "";
    let body = r#"
    
# get call payload size
calldatasize
# store payload to mem 0x00
push1 0x00
push1 0x00
calldatacopy

# out size
# out off
push1 0x20
push1 0xA0

# in size
# in off
calldatasize
push1 0x00

# value
push1 0x00

# dst (resolve_address precompile)
push20 0xfe00000000000000000000000000000000000001

# gas
push1 0x00

call

# write exit code memory
push1 0x00 # offset
mstore8

returndatasize
push1 0x00 # offset
push1 0x01 # dest offset
returndatacopy

returndatasize
push1 0x01
add
push1 0x00
return
"#;
    asm::new_contract("native_precompiles", init, body).unwrap()
}

#[test]
fn test_native_lookup_delegated_address() {
    let bytecode = {
        let init = "";
        let body = r#"
    
# get call payload size
calldatasize
# store payload to mem 0x00
push1 0x00
push1 0x00
calldatacopy

push1 0x20   # out size
push1 0xA0   # out off
calldatasize # in size
push1 0x00   # in off
push1 0x00   # value
# dst (lookup_delegated_address precompile)
push20 0xfe00000000000000000000000000000000000002
push1 0x00   # gas
call

# copy result to mem 0x00
returndatasize
push1 0x00
push1 0x00
returndatacopy
# return
returndatasize
push1 0x00
return
"#;

        asm::new_contract("native_lookup_delegated_address", init, body).unwrap()
    };
    let mut rt = util::construct_and_verify(bytecode);

    // f0 10101 is an EVM actor
    let evm_target = FILAddress::new_id(10101);
    let evm_del = EthAddress(util::CONTRACT_ADDRESS).try_into().unwrap();
    rt.add_delegated_address(evm_target, evm_del);

    fn test_reslove(rt: &mut MockRuntime, id: FILAddress, expected: Vec<u8>) {
        rt.expect_gas_available(10_000_000_000u64);
        let result = util::invoke_contract(rt, &id_to_vec(&id));
        rt.verify();
        assert_eq!(expected, result.as_slice());
        rt.reset();
    }

    test_reslove(&mut rt, evm_target, evm_del.to_bytes());
    test_reslove(&mut rt, FILAddress::new_id(11111), Vec::new());
}

#[test]
fn test_precompile_failure() {
    let bytecode = resolve_address_contract();
    let mut rt = util::construct_and_verify(bytecode);

    // invalid input fails
    rt.expect_gas_available(10_000_000_000u64);
    let result = util::invoke_contract(&mut rt, &[0xff; 32]);
    rt.verify();
    assert_eq!(&[0u8], result.as_slice());
    rt.reset();

    // not found succeeds with empty
    rt.expect_gas_available(10_000_000_000u64);
    let result = util::invoke_contract(&mut rt, &U256::from(111).to_bytes());
    rt.verify();
    assert_eq!(&[1u8], result.as_slice());
    rt.reset();
}
