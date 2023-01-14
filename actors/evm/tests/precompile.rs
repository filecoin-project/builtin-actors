mod asm;

use std::fmt::Debug;

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

fn precompile_address(prefix: u8, index: u8) -> EthAddress {
    let mut buf = [0u8; 20];
    buf[0] = prefix;
    buf[19] = index;
    EthAddress(buf)
}

#[repr(u8)]
#[derive(Debug, PartialEq, Eq)]
enum PrecompileExit {
    Reverted = 0,
    Success = 1,
}

#[repr(u8)]
#[derive(Debug)]
pub enum NativePrecompile {
    ResolveAddress = 1,
    LookupDelegatedAddress = 2,
    CallActor = 3,
    GetActorType = 4,
}

impl NativePrecompile {
    fn as_address(&self) -> EthAddress {
        precompile_address(0xfe, *self as u8)
    }
}

struct PrecompileTest {
    pub expected_return: Vec<u8>,
    pub expected_exit_code: PrecompileExit,
    pub precompile_address: EthAddress,
    pub output_size: u32,
    pub input: Vec<u8>,
    pub gas_avaliable: u64,
}

impl Debug for PrecompileTest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PrecompileTest")
            .field("expected_exit_code", &self.expected_exit_code)
            .field("precompile_address", &self.precompile_address)
            .field("input", &hex::encode(&self.input))
            .field("expected_return", &hex::encode(&self.expected_return))
            .field("output_size", &self.output_size)
            .field("gas_avaliable", &self.gas_avaliable)
            .finish()
    }
}

impl PrecompileTest {
    fn run_test(&self, rt: &mut MockRuntime) {
        rt.expect_gas_available(self.gas_avaliable);
        log::trace!("{:#?}", &self);
        // first byte is precompile number, second is output buffer size, rest is input to precompile
        let result = util::invoke_contract(
            rt,
            &[
                self.precompile_address.as_evm_word().to_bytes().to_vec(),
                U256::from(self.output_size).to_bytes().to_vec(),
                self.input.clone(),
            ]
            .concat(),
        );
        log::trace!("returned: {:?}", hex::encode(&result));
        rt.verify();

        let returned_exit = match result[0] {
            0 => PrecompileExit::Reverted,
            1 => PrecompileExit::Success,
            _ => panic!("Expected call to give either 1 or 0, this is a bug!"),
        };
        assert_eq!(self.expected_exit_code, returned_exit);
        assert_eq!(&self.expected_return, &result[1..]);
        rt.reset();
    }

    fn test_runner_bytecode() -> Vec<u8> {
        Self::test_runner_bytecode_transfer_value(0)
    }
    fn test_runner_bytecode_transfer_value(value: u64) -> Vec<u8> {
        let init = "";
        let body = format!(
            r#"
# store entire input to mem 0x00
calldatasize
push1 0x00 # input offset
push1 0x00 # dst offset
calldatacopy

# out size
push1 0x20 # second word of input
mload

# out off
push2 0xA000

# in size
push1 0x40 # two words
calldatasize
sub
# in off
push1 0x40 # two words

# value
%push({value})

# precompile address
push1 0x00 # first word of input is precompile
mload

# gas
push1 0x00

call

# write exit code first byte of memory
push1 0x00 # offset
mstore8

# write precompile return to memory
returndatasize
push1 0x00 # input offset
push1 0x01 # dst offset (plus 1 to accommodate exit code)
returndatacopy

# size
returndatasize
push1 0x01
add
# offset
push1 0x00
return
"#
        );

        asm::new_contract("precompile_tester", init, &body).unwrap()
    }
}

#[test]
fn test_native_actor_type() {
    use evm::interpreter::precompiles::NativeType;

    let mut rt = util::construct_and_verify(PrecompileTest::test_runner_bytecode());

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
            precompile_address: NativePrecompile::GetActorType.as_address(),
            input: id_to_vec(&id),
            output_size: 32,
            expected_exit_code: PrecompileExit::Success,
            expected_return: U256::from(expected as u32).to_bytes().to_vec(),
            gas_avaliable: 10_000_000_000,
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
            precompile_address: NativePrecompile::GetActorType.as_address(),
            input,
            output_size: 32,
            expected_exit_code: PrecompileExit::Reverted,
            expected_return: vec![],
            gas_avaliable: 10_000_000_000,
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
    let mut rt = util::construct_and_verify(PrecompileTest::test_runner_bytecode());

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
            precompile_address: NativePrecompile::LookupDelegatedAddress.as_address(),
            input: id_to_vec(&id),
            output_size: 32,
            expected_exit_code: PrecompileExit::Success,
            expected_return: expected,
            gas_avaliable: 10_000_000_000,
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
        let input = {
            let addr = addr.to_bytes();
            let mut v = U256::from(addr.len()).to_bytes().to_vec();
            v.extend_from_slice(&addr);
            v
        };
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

    // valid with extra padding
    rt.expect_gas_available(10_000_000_000u64);
    let input = {
        let addr = evm_del.to_bytes();
        // address length to read
        let mut v = U256::from(addr.len()).to_bytes().to_vec();
        // address itself
        v.extend_from_slice(&addr);
        // extra padding
        v.extend_from_slice(&[0; 10]);
        v
    };
    let result = util::invoke_contract(&mut rt, &input);
    rt.verify();
    assert_eq!(id_to_vec(&evm_target), &result[1..]);
    assert_eq!(1, result[0]);
    rt.reset();

    // valid but needs padding
    rt.expect_gas_available(10_000_000_000u64);
    let input = {
        // EVM f4 but subaddress len is 12 bytes
        // FEEDFACECAFEBEEF00000000
        let addr = FILAddress::new_delegated(10, &util::CONTRACT_ADDRESS[..12]).unwrap();
        let addr = addr.to_bytes();

        let read_len = addr.len() + 8;
        let mut v = U256::from(read_len).to_bytes().to_vec();
        // address itself
        v.extend_from_slice(&addr);
        v
    };
    let result = util::invoke_contract(&mut rt, &input);
    rt.verify();
    assert_eq!(id_to_vec(&evm_target), &result[1..]);
    assert_eq!(1, result[0]);
    rt.reset();

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
    let mut rt = util::construct_and_verify(PrecompileTest::test_runner_bytecode_transfer_value(1));
    rt.set_balance(TokenAmount::from_atto(100));
    // test invalid precompile address
    for (prefix, index) in [(0x00, 0xff), (0xfe, 0xff)] {
        let addr = precompile_address(prefix, index);
        let test = PrecompileTest {
            precompile_address: addr,
            input: vec![0xff; 32], // garbage input should change nothing
            output_size: 32,
            expected_exit_code: PrecompileExit::Success,
            expected_return: vec![],
            gas_avaliable: 10_000_000_000,
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
    let input = {
        let addr = FILAddress::new_delegated(111, b"foo").unwrap().to_bytes();
        // first word is len
        let mut v = U256::from(addr.len()).to_bytes().to_vec();
        // then addr
        v.extend_from_slice(&addr);
        v
    };
    let result = util::invoke_contract(&mut rt, &input);
    rt.verify();
    assert_eq!(&[1u8], result.as_slice());
    rt.reset();
}
