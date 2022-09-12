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

#[test]
fn test_extcodesize() {
    let bytecode: RawBytes = {
        let init = "";
        let body = r#"
        # get code size of address f088
        push20 0xff00000000000000000000000000000000000088
        extcodesize
        # store size at location 0x00
        push1 0x00
        mstore
        # return 0x00..0x20
        push1 0x20
        push1 0x00
        return
        "#;

        asm::new_contract("extcodesize", init, body).unwrap()
    }
    .into();

    let mut rt = MockRuntime::default();
    rt.expect_validate_caller_any();
    let params = evm::ConstructorParams { bytecode, input_data: RawBytes::default() };
    let result = rt
        .call::<evm::EvmContractActor>(
            evm::Method::Constructor as u64,
            &RawBytes::serialize(params).unwrap(),
        )
        .unwrap();
    expect_empty(result);
    rt.verify();

    // 0x88 is an EVM actor
    let target = FILAddress::new_id(0x88);
    rt.actor_code_cids.insert(target, *EVM_ACTOR_CODE_ID);

    // a random CID
    let bytecode_cid = Cid::try_from("baeaikaia").unwrap();
    let other_bytecode = vec![0x01, 0x02, 0x03, 0x04];
    rt.store.put_keyed(&bytecode_cid, &other_bytecode.as_slice()).unwrap();

    rt.expect_validate_caller_any();
    rt.expect_send(
        target,
        evm::Method::GetBytecode as u64,
        Default::default(),
        TokenAmount::zero(),
        RawBytes::serialize(&bytecode_cid).unwrap(),
        ExitCode::OK,
    );

    let result = rt
        .call::<evm::EvmContractActor>(evm::Method::InvokeContract as u64, &Default::default())
        .unwrap();

    assert_eq!(U256::from_big_endian(&result), U256::from(0x04));
}

#[test]
fn test_extcodehash() {
    let bytecode: RawBytes = {
        let init = "";
        let body = r#"
        # get code hash of address f088
        push20 0xff00000000000000000000000000000000000088
        extcodehash
        # store size at location 0x00
        push1 0x00
        mstore
        # return 0x00..0x20
        push1 0x20
        push1 0x00
        return
        "#;

        asm::new_contract("extcodehash", init, body).unwrap()
    }
    .into();

    let mut rt = MockRuntime::default();
    rt.expect_validate_caller_any();
    let params = evm::ConstructorParams { bytecode, input_data: RawBytes::default() };
    let result = rt
        .call::<evm::EvmContractActor>(
            evm::Method::Constructor as u64,
            &RawBytes::serialize(params).unwrap(),
        )
        .unwrap();
    expect_empty(result);
    rt.verify();

    // 0x88 is an EVM actor
    let target = FILAddress::new_id(0x88);
    rt.actor_code_cids.insert(target, *EVM_ACTOR_CODE_ID);

    // a random CID
    let bytecode_cid =
        Cid::try_from("bafy2bzacecu7n7wbtogznrtuuvf73dsz7wasgyneqasksdblxupnyovmtwxxu").unwrap();

    rt.expect_validate_caller_any();
    rt.expect_send(
        target,
        evm::Method::GetBytecode as u64,
        Default::default(),
        TokenAmount::zero(),
        RawBytes::serialize(&bytecode_cid).unwrap(),
        ExitCode::OK,
    );

    let result = rt
        .call::<evm::EvmContractActor>(evm::Method::InvokeContract as u64, &Default::default())
        .unwrap();

    assert_eq!(U256::from_big_endian(&result), U256::from(&bytecode_cid.hash().digest()[..32]));
}

#[test]
fn test_extcodecopy() {
    let bytecode: RawBytes = {
        let init = "";
        let body = r#"
        push1 0xff
        push1 0x00
        push1 0x00
        push20 0xff00000000000000000000000000000000000088
        extcodecopy
        # return 0x00..0x04
        push1 0x04
        push1 0x00
        return
        "#;

        asm::new_contract("extcodecopy", init, body).unwrap()
    }
    .into();

    let mut rt = MockRuntime::default();
    rt.expect_validate_caller_any();
    let params = evm::ConstructorParams { bytecode, input_data: RawBytes::default() };
    let result = rt
        .call::<evm::EvmContractActor>(
            evm::Method::Constructor as u64,
            &RawBytes::serialize(params).unwrap(),
        )
        .unwrap();
    expect_empty(result);
    rt.verify();

    // 0x88 is an EVM actor
    let target = FILAddress::new_id(0x88);
    rt.actor_code_cids.insert(target, *EVM_ACTOR_CODE_ID);

    // a random CID
    let bytecode_cid = Cid::try_from("baeaikaia").unwrap();
    let other_bytecode = vec![0x01, 0x02, 0x03, 0x04];
    rt.store.put_keyed(&bytecode_cid, &other_bytecode.as_slice()).unwrap();

    rt.expect_validate_caller_any();
    rt.expect_send(
        target,
        evm::Method::GetBytecode as u64,
        Default::default(),
        TokenAmount::zero(),
        RawBytes::serialize(&bytecode_cid).unwrap(),
        ExitCode::OK,
    );

    let result = rt
        .call::<evm::EvmContractActor>(evm::Method::InvokeContract as u64, &Default::default())
        .unwrap();

    assert_eq!(other_bytecode.as_slice(), result.bytes());
}
