mod asm;

use cid::Cid;
use evm::interpreter::U256;
use fil_actor_evm as evm;
use fil_actors_runtime::test_utils::*;
use fil_actors_runtime::ActorError;
use fvm_ipld_blockstore::Blockstore;
use fvm_ipld_encoding::RawBytes;
use fvm_shared::address::Address;

mod util;

#[test]
fn basic_contract_construction_and_invocation() {
    let bytecode = hex::decode(include_str!("contracts/simplecoin.hex")).unwrap();
    let contract = Address::new_id(100);

    let mut rt = util::init_construct_and_verify(bytecode, |rt| {
        rt.actor_code_cids.insert(contract, *EVM_ACTOR_CODE_ID);
        rt.set_origin(contract);
    });

    // invoke contract -- getBalance
    // first we invoke without specifying an address, so it would be the system actor and have
    // a balance of 0

    let mut solidity_params = vec![];
    solidity_params.append(&mut hex::decode("f8b2cb4f").unwrap()); // function selector
                                                                   // caller id address in U256 form
    let mut arg0 = vec![0u8; 32];
    solidity_params.append(&mut arg0);

    let result = util::invoke_contract(&mut rt, &solidity_params);
    assert_eq!(U256::from_big_endian(&result), U256::from(0));

    // invoke contract -- getBalance
    // now we invoke with the owner address, which should have a balance of 10k
    let mut solidity_params = vec![];
    solidity_params.append(&mut hex::decode("f8b2cb4f").unwrap()); // function selector
                                                                   // caller id address in U256 form
    let mut arg0 = vec![0u8; 32];
    arg0[12] = 0xff; // it's an ID address, so we enable the flag
    arg0[31] = 100; // the owner address
    solidity_params.append(&mut arg0);

    let result = util::invoke_contract(&mut rt, &solidity_params);
    assert_eq!(U256::from_big_endian(&result), U256::from(10000));
}

#[test]
fn basic_get_bytecode() {
    let (init_code, verbatim_body) = {
        let init = "";
        let body = r#"
# get call payload size
push1 0x20
calldatasize
sub
# store payload to mem 0x00
push1 0x20
push1 0x00
calldatacopy
return
"#;

        let body_bytecode = {
            let mut ret = Vec::new();
            let mut ingest = etk_asm::ingest::Ingest::new(&mut ret);
            ingest.ingest("body", body).unwrap();
            ret
        };

        (asm::new_contract("get_bytecode", init, body).unwrap(), body_bytecode)
    };

    let mut rt = util::construct_and_verify(init_code);

    rt.reset();
    rt.expect_validate_caller_any();
    let returned_bytecode_cid: Cid = rt
        .call::<evm::EvmContractActor>(evm::Method::GetBytecode as u64, &Default::default())
        .unwrap()
        .deserialize()
        .unwrap();
    rt.verify();

    let bytecode = rt.store.get(&returned_bytecode_cid).unwrap().unwrap();

    assert_eq!(bytecode.as_slice(), verbatim_body.as_slice());
}

#[test]
fn basic_get_storage_at() {
    let init_code = {
        // Initialize storage entry on key 0x8965 during init.
        let init = r"
push2 0xfffa
push2 0x8965
sstore";
        let body = r#"return"#;

        asm::new_contract("get_storage_at", init, body).unwrap()
    };

    let mut rt = util::construct_and_verify(init_code);

    rt.reset();
    let params = evm::GetStorageAtParams { storage_key: 0x8965.into() };

    let sender = Address::new_id(0); // zero address because this method is not invokable on-chain
    rt.expect_validate_caller_addr(vec![sender]);
    rt.caller = sender;

    //
    // Get the storage key that was initialized in the init code.
    //
    let value: U256 = rt
        .call::<evm::EvmContractActor>(
            evm::Method::GetStorageAt as u64,
            &RawBytes::serialize(params).unwrap(),
        )
        .unwrap()
        .deserialize()
        .unwrap();
    rt.verify();
    rt.reset();

    assert_eq!(U256::from(0xfffa), value);

    //
    // Get a storage key that doesn't exist.
    //
    let params = evm::GetStorageAtParams { storage_key: 0xaaaa.into() };

    rt.expect_validate_caller_addr(vec![sender]);
    let ret = rt.call::<evm::EvmContractActor>(
        evm::Method::GetStorageAt as u64,
        &RawBytes::serialize(params).unwrap(),
    );
    rt.verify();

    assert_eq!(ActorError::not_found("storage key not found".to_string()), ret.err().unwrap());
}
