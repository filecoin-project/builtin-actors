mod asm;

use cid::Cid;
use evm::interpreter::U256;
use fil_actor_evm as evm;
use fil_actors_runtime::test_utils::*;
use fvm_ipld_blockstore::Blockstore;
use fvm_ipld_encoding::RawBytes;
use fvm_shared::address::Address as FILAddress;
use fvm_shared::bigint::Zero;
use fvm_shared::econ::TokenAmount;
use fvm_shared::error::ExitCode;
use fvm_shared::{IDENTITY_HASH, IPLD_RAW};
use lazy_static::lazy_static;

mod util;

lazy_static! {
    pub static ref DUMMY_ACTOR_CODE_ID: Cid =
        Cid::new_v1(IPLD_RAW, Multihash::wrap(IDENTITY_HASH, b"foobarboxy").unwrap());
}

#[test]
fn test_extcodesize() {
    let bytecode = {
        let init = "";
        let body = r#"        
%dispatch_begin()
%dispatch(0x00, evm_size)
%dispatch(0x01, native_size)
# TODO update after real account abstraction lands
%dispatch(0x02, evm_account)
%dispatch(0x03, native_account)
%dispatch_end()

evm_size:
    jumpdest
    # get code size of address f088
    push20 0xff00000000000000000000000000000000000088
    extcodesize
    %return_stack_word()

native_size:
    jumpdest
    # native actor ID
    push20 0xff00000000000000000000000000000000000089
    extcodesize
    %return_stack_word()

evm_account: 
    jumpdest
    # evm account
    push20 0xff00000000000000000000000000000000000101
    extcodesize
    %return_stack_word()

native_account:
    jumpdest
    # native actor ID
    push20 0xff00000000000000000000000000000000000102
    extcodesize
    %return_stack_word()
"#;

        asm::new_contract("extcodesize", init, body).unwrap()
    };

    let mut rt = util::construct_and_verify(bytecode);

    // a fake CID
    let bytecode_cid = Cid::try_from("baeaikaia").unwrap();
    let bytecode = vec![0x01, 0x02, 0x03, 0x04];
    rt.store.put_keyed(&bytecode_cid, bytecode.as_slice()).unwrap();

    // 0x88 is an EVM actor
    let evm_contract = FILAddress::new_id(0x88);
    rt.set_address_actor_type(evm_contract, *EVM_ACTOR_CODE_ID);

    // 0x89 is a native actor
    let native_actor = FILAddress::new_id(0x89);
    rt.set_address_actor_type(native_actor, *DUMMY_ACTOR_CODE_ID);

    // 0x0101 is an EVM EOA account
    let evm_account = FILAddress::new_id(0x0101);
    // TODO this is part of the account abstraction hack, where embryos are magically accounts
    rt.set_address_actor_type(evm_account, *EMBRYO_ACTOR_CODE_ID);

    // 0x0102 is a native account
    let native_account = FILAddress::new_id(0x0102);
    rt.set_address_actor_type(native_account, *ACCOUNT_ACTOR_CODE_ID);

    // evm actor
    let method = util::dispatch_num_word(0);
    let expected = U256::from(0x04);
    {
        rt.expect_send(
            evm_contract,
            evm::Method::GetBytecode as u64,
            Default::default(),
            TokenAmount::zero(),
            RawBytes::serialize(&bytecode_cid).unwrap(),
            ExitCode::OK,
        );

        let result = util::invoke_contract(&mut rt, &method);
        rt.verify();
        assert_eq!(U256::from_big_endian(&result), expected);
        rt.reset();
    }

    // native actor
    let method = util::dispatch_num_word(1);
    let expected = U256::from(0x01);
    {
        let result = util::invoke_contract(&mut rt, &method);
        rt.verify();
        assert_eq!(U256::from_big_endian(&result), expected);
        rt.reset();
    }

    // EVM account
    let method = util::dispatch_num_word(2);
    let expected = U256::from(0x00);
    {
        let result = util::invoke_contract(&mut rt, &method);
        rt.verify();
        assert_eq!(U256::from_big_endian(&result), expected);
        rt.reset();
    }

    // native account
    let method = util::dispatch_num_word(3);
    let expected = U256::from(0x00);
    {
        let result = util::invoke_contract(&mut rt, &method);
        rt.verify();
        assert_eq!(U256::from_big_endian(&result), expected);
        rt.reset();
    }
}

#[test]
fn test_extcodehash() {
    let bytecode = {
        let init = "";
        let body = r#"
%dispatch_begin()
%dispatch(0x00, evm_contract)
%dispatch(0x01, native_actor)
%dispatch_end()
        
evm_contract:
    jumpdest
    # get code hash of address 0x88
    push20 0xff00000000000000000000000000000000000088
    extcodehash
    %return_stack_word()

native_actor:
    jumpdest
    # get code hash of address 0x89
    push20 0xff00000000000000000000000000000000000089
    extcodehash
    %return_stack_word()

"#;

        asm::new_contract("extcodehash", init, body).unwrap()
    };

    let mut rt = util::construct_and_verify(bytecode);

    // 0x88 is an EVM actor
    let evm_target = FILAddress::new_id(0x88);
    rt.set_address_actor_type(evm_target, *EVM_ACTOR_CODE_ID);

    // 0x88 is an EVM actor
    let native_target = FILAddress::new_id(0x89);
    rt.set_address_actor_type(native_target, *DUMMY_ACTOR_CODE_ID);

    // a random CID
    let bytecode_cid =
        Cid::try_from("bafy2bzacecu7n7wbtogznrtuuvf73dsz7wasgyneqasksdblxupnyovmtwxxu").unwrap();

    rt.expect_send(
        evm_target,
        evm::Method::GetBytecode as u64,
        Default::default(),
        TokenAmount::zero(),
        RawBytes::serialize(&bytecode_cid).unwrap(),
        ExitCode::OK,
    );

    let result = util::invoke_contract(&mut rt, &util::dispatch_num_word(0));
    rt.verify();
    assert_eq!(U256::from_big_endian(&result), U256::from(&bytecode_cid.hash().digest()[..32]));
    rt.reset();

    util::invoke_contract_expect_abort(
        &mut rt,
        &util::dispatch_num_word(1),
        evm::interpreter::StatusCode::InvalidArgument(
            "Cannot invoke EXTCODEHASH for non-EVM actor.".to_string(),
        ),
    );
}

#[test]
fn test_extcodecopy() {
    let bytecode = {
        let init = "";
        let body = r#"

%dispatch_begin()
%dispatch(0x00, evm_contract)
%dispatch(0x01, native_actor)
%dispatch(0x02, invalid_address)
%dispatch_end()

evm_contract:
    jumpdest
    push1 0xff
    push1 0x00
    push1 0x00
    push20 0xff00000000000000000000000000000000000088
    extcodecopy
    # return 0x00..0x04
    push1 0x04
    push1 0x00
    return

native_actor:
    jumpdest
    push1 0xff
    push1 0x00
    push1 0x00
    push20 0xff00000000000000000000000000000000000089
    extcodecopy
    # return 0x00..0x01
    push1 0x01
    push1 0x00
    return

invalid_address:
    jumpdest
    push1 0xff
    push1 0x00
    push1 0x00
    push20 0xff000000000000000000000000000000000000ff
    extcodecopy
    # return 0x00..0x20
    push1 0x20
    push1 0x00
    return

        "#;

        asm::new_contract("extcodecopy", init, body).unwrap()
    };

    let mut rt = util::construct_and_verify(bytecode);

    // 0x88 is an EVM actor
    let evm_target = FILAddress::new_id(0x88);
    rt.set_address_actor_type(evm_target, *EVM_ACTOR_CODE_ID);

    // 0x89 is a native actor
    let native_target = FILAddress::new_id(0x89);
    rt.set_address_actor_type(native_target, *DUMMY_ACTOR_CODE_ID);

    // a random CID
    let bytecode_cid = Cid::try_from("baeaikaia").unwrap();
    let other_bytecode = vec![0x01, 0x02, 0x03, 0x04];
    rt.store.put_keyed(&bytecode_cid, other_bytecode.as_slice()).unwrap();

    rt.expect_send(
        evm_target,
        evm::Method::GetBytecode as u64,
        Default::default(),
        TokenAmount::zero(),
        RawBytes::serialize(&bytecode_cid).unwrap(),
        ExitCode::OK,
    );

    let result = util::invoke_contract(&mut rt, &util::dispatch_num_word(0));
    rt.verify();
    assert_eq!(other_bytecode.as_slice(), result);
    rt.reset();

    // calling code copy on native actors return "invalid" instruction from EIP-141
    let result = util::invoke_contract(&mut rt, &util::dispatch_num_word(1));
    rt.verify();
    assert_eq!(vec![0xFE], result);
    rt.reset();

    // invalid addresses are flattened
    let result = util::invoke_contract(&mut rt, &util::dispatch_num_word(2));
    rt.verify();
    assert_eq!(U256::from_big_endian(&result), U256::from(0));
}
