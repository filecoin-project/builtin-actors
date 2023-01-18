mod asm;

use evm::interpreter::{address::EthAddress, U256};
use fil_actor_evm as evm;
use fil_actors_runtime::{
    test_utils::{
        new_bls_addr, MockRuntime, ACCOUNT_ACTOR_CODE_ID, EAM_ACTOR_CODE_ID, EVM_ACTOR_CODE_ID,
        MARKET_ACTOR_CODE_ID, MINER_ACTOR_CODE_ID, MULTISIG_ACTOR_CODE_ID,
        PLACEHOLDER_ACTOR_CODE_ID,
    },
    EAM_ACTOR_ID,
};
use fvm_shared::{address::Address as FILAddress, econ::TokenAmount, error::ExitCode, METHOD_SEND};

mod util;
use util::{id_to_vec, NativePrecompile, PrecompileExit, PrecompileTest};

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

fn tester_bytecode() -> Vec<u8> {
    let (init, body) = util::PrecompileTest::test_runner_assembly();
    asm::new_contract("precompile-tester", &init, &body).unwrap()
}

#[test]
fn test_native_actor_type() {
    use evm::interpreter::precompiles::NativeType;

    let mut rt = util::construct_and_verify(tester_bytecode());

    // 0x88 is an EVM actor
    let evm_target = FILAddress::new_id(0x88);
    rt.set_address_actor_type(evm_target, *EVM_ACTOR_CODE_ID);

    // f0 10 is the EAM actor (System)
    let eam_target = FILAddress::new_id(10);
    rt.set_address_actor_type(eam_target, *EAM_ACTOR_CODE_ID);

    // f0 7 is the Market actor (System)
    let market_target = FILAddress::new_id(7);
    rt.set_address_actor_type(market_target, *MARKET_ACTOR_CODE_ID);

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
        let test = PrecompileTest {
            precompile_address: NativePrecompile::GetActorType.eth_address(),
            output_size: 32,
            expected_exit_code: PrecompileExit::Success,
            gas_avaliable: 10_000_000_000,
            call_op: util::PrecompileCallOpcode::Call(0),
            input: id_to_vec(&id),
            expected_return: U256::from(expected as u32).to_bytes().to_vec(),
        };
        test.run_test(rt);
    }

    test_type(&mut rt, evm_target, NativeType::EVMContract);
    test_type(&mut rt, eam_target, NativeType::System);
    test_type(&mut rt, market_target, NativeType::System);
    test_type(&mut rt, account_target, NativeType::Account);
    test_type(&mut rt, placeholder_target, NativeType::Placeholder);
    test_type(&mut rt, miner_target, NativeType::StorageProvider);
    test_type(&mut rt, other_target, NativeType::OtherTypes);
    test_type(&mut rt, FILAddress::new_id(10101), NativeType::NonExistent);

    // invalid id parameter (over)
    fn test_type_invalid(rt: &mut MockRuntime, input: Vec<u8>) {
        let test = PrecompileTest {
            precompile_address: NativePrecompile::GetActorType.eth_address(),
            output_size: 32,
            expected_exit_code: PrecompileExit::Reverted,
            gas_avaliable: 10_000_000_000,
            call_op: util::PrecompileCallOpcode::Call(0),
            expected_return: vec![],
            input,
        };
        test.run_test(rt);
    }

    // extra bytes
    test_type_invalid(&mut rt, vec![0xff; 64]);
    // single byte get padded and is invalid
    test_type_invalid(&mut rt, vec![0xff]);

    // VERY weird and NOBODY should depend on this, but this is expected behavior soo
    // ¯\_(ツ)_/¯
    {
        // f0 (0xff00)
        let padded_target = FILAddress::new_id(0xff00);
        rt.set_address_actor_type(padded_target, *EVM_ACTOR_CODE_ID);

        // not enough bytes (but still valid id when padded)
        let mut input = vec![0u8; 31];
        input[30] = 0xff; // will get padded to 0xff00
        test_type(&mut rt, evm_target, NativeType::EVMContract);
    }
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
    let mut rt = util::construct_and_verify(tester_bytecode());

    // f0 10101 is an EVM actor
    let evm_target = FILAddress::new_id(10101);
    let evm_del = EthAddress(util::CONTRACT_ADDRESS).try_into().unwrap();
    rt.add_delegated_address(evm_target, evm_del);

    // f0 10111 is an actor with a non-evm delegate address
    let unknown_target = FILAddress::new_id(10111);
    let unknown_del = FILAddress::new_delegated(1234, "foobarboxy".as_bytes()).unwrap();
    rt.add_delegated_address(unknown_target, unknown_del);

    fn test_lookup_address(rt: &mut MockRuntime, id: FILAddress, expected: Vec<u8>) {
        let test = PrecompileTest {
            precompile_address: NativePrecompile::LookupDelegatedAddress.eth_address(),
            output_size: 32,
            expected_exit_code: PrecompileExit::Success,
            gas_avaliable: 10_000_000_000,
            call_op: util::PrecompileCallOpcode::Call(0),
            expected_return: expected,
            input: id_to_vec(&id),
        };

        test.run_test(rt);
    }

    test_lookup_address(&mut rt, evm_target, evm_del.to_bytes());
    test_lookup_address(&mut rt, unknown_target, unknown_del.to_bytes());
    test_lookup_address(&mut rt, FILAddress::new_id(11111), Vec::new());
}

#[test]
fn test_resolve_delegated() {
    let bytecode = resolve_address_contract();
    let mut rt = util::construct_and_verify(bytecode);

    // EVM actor
    let evm_target = FILAddress::new_id(10101);
    let evm_del = EthAddress(util::CONTRACT_ADDRESS).try_into().unwrap();
    rt.add_delegated_address(evm_target, evm_del);

    // Actor with a non-evm delegate address
    let unknown_target = FILAddress::new_id(10111);
    let unknown_del = FILAddress::new_delegated(1234, "foobarboxy".as_bytes()).unwrap();
    rt.add_delegated_address(unknown_target, unknown_del);

    // Non-bound f4 address
    let unbound_del = FILAddress::new_delegated(0xffff, "foobarboxybeef".as_bytes()).unwrap();

    // Actor with a secp address
    let secp_target = FILAddress::new_id(10112);
    let secp = {
        let mut protocol = vec![1u8];
        let payload = [0xff; 20];
        protocol.extend_from_slice(&payload);
        FILAddress::from_bytes(&protocol).unwrap()
    };
    rt.add_id_address(secp, secp_target);

    // Actor with a bls address
    let bls_target = FILAddress::new_id(10113);
    let bls = new_bls_addr(123);
    rt.add_id_address(bls, bls_target);

    fn test_resolve(rt: &mut MockRuntime, addr: FILAddress, expected: Vec<u8>) {
        rt.expect_gas_available(10_000_000_000u64);
        let input = addr.to_bytes();
        let result = util::invoke_contract(rt, &input);
        rt.verify();
        assert_eq!(expected, &result[1..]);
        assert_eq!(1, result[0]);
        rt.reset();
    }

    test_resolve(&mut rt, evm_del, id_to_vec(&evm_target));
    test_resolve(&mut rt, unknown_del, id_to_vec(&unknown_target));
    test_resolve(&mut rt, secp, id_to_vec(&secp_target));
    test_resolve(&mut rt, bls, id_to_vec(&bls_target));
    // not found
    test_resolve(&mut rt, unbound_del, vec![]);

    // invalid first param fails
    rt.expect_gas_available(10_000_000_000u64);
    let result = util::invoke_contract(&mut rt, &[0xff; 1]);
    rt.verify();
    assert_eq!(&[0u8], result.as_slice());
    rt.reset();

    // invalid second param fails
    rt.expect_gas_available(10_000_000_000u64);
    let input = {
        // first word is len
        let mut v = U256::from(5).to_bytes().to_vec();
        // then addr
        v.extend_from_slice(&[0, 0, 0xff]);
        v
    };
    let result = util::invoke_contract(&mut rt, &input);
    rt.verify();
    assert_eq!(&[0u8], result.as_slice());
    rt.reset();
}

#[test]
fn test_precompile_transfer() {
    let (init, body) = util::PrecompileTest::test_runner_assembly();

    let mut rt =
        util::construct_and_verify(asm::new_contract("precompile-tester", &init, &body).unwrap());
    rt.set_balance(TokenAmount::from_atto(100));
    // test invalid precompile address
    for (prefix, index) in [(0x00, 0xff), (0xfe, 0xff)] {
        let addr = util::precompile_address(prefix, index);
        let test = PrecompileTest {
            precompile_address: addr,
            output_size: 32,
            expected_exit_code: PrecompileExit::Success,
            gas_avaliable: 10_000_000_000,
            call_op: util::PrecompileCallOpcode::Call(1),
            input: vec![0xff; 32],
            expected_return: vec![],
        };
        let fil_addr = FILAddress::new_delegated(EAM_ACTOR_ID, addr.as_ref()).unwrap();
        rt.expect_send(fil_addr, METHOD_SEND, None, TokenAmount::from_atto(1), None, ExitCode::OK);
        test.run_test(&mut rt);
    }
    assert_eq!(rt.get_balance(), TokenAmount::from_atto(98));
}

#[test]
fn test_precompile_failure() {
    // TODO: refactor these to be more clear

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
    let input = FILAddress::new_delegated(111, b"foo").unwrap().to_bytes();
    let result = util::invoke_contract(&mut rt, &input);
    rt.verify();
    assert_eq!(&[1u8], result.as_slice());
    rt.reset();
}
