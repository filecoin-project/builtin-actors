mod asm;

use evm::interpreter::U256;
use fil_actor_evm as evm;
use fil_actors_runtime::test_utils::{
    MockRuntime, ACCOUNT_ACTOR_CODE_ID, EAM_ACTOR_CODE_ID, EMBRYO_ACTOR_CODE_ID, EVM_ACTOR_CODE_ID,
    MINER_ACTOR_CODE_ID, MULTISIG_ACTOR_CODE_ID,
};
use fvm_shared::address::Address as FILAddress;

mod util;

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
push1 0x0c

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

        asm::new_contract("native_precompiles", init, body).unwrap()
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

    // f0 102 is an embryo
    let embryo_target = FILAddress::new_id(102);
    rt.set_address_actor_type(embryo_target, *EMBRYO_ACTOR_CODE_ID);

    // f0 103 is a storage provider
    let miner_target = FILAddress::new_id(103);
    rt.set_address_actor_type(miner_target, *MINER_ACTOR_CODE_ID);

    // f0 104 is a multisig
    let other_target = FILAddress::new_id(104);
    rt.set_address_actor_type(other_target, *MULTISIG_ACTOR_CODE_ID);

    fn id_to_vec(src: &FILAddress) -> Vec<u8> {
        U256::from(src.id().unwrap()).to_bytes().to_vec()
    }

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
    test_type(&mut rt, embryo_target, NativeType::Embryo);
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
