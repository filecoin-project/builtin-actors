mod asm;

use cid::Cid;
use fil_actor_evm as evm;
use fil_actors_evm_shared::uints::U256;
use fil_actors_runtime::test_utils::*;
use fvm_ipld_blockstore::Blockstore;
use fvm_ipld_encoding::ipld_block::IpldBlock;
use fvm_shared::address::Address;

mod util;

#[test]
fn basic_contract_construction_and_invocation_fe_lang() {
    let bytecode =
        hex::decode(include_str!("contracts/output/FeSimplecoin/FeSimplecoin.bin")).unwrap();
    simplecoin_test(bytecode);
}

#[test]
fn basic_contract_construction_and_invocation() {
    let bytecode = hex::decode(include_str!("contracts/simplecoin.hex")).unwrap();
    simplecoin_test(bytecode);
}

fn simplecoin_test(bytecode: Vec<u8>) {
    let contract = Address::new_id(100);

    let rt = util::init_construct_and_verify(bytecode, |rt| {
        rt.actor_code_cids.borrow_mut().insert(contract, *EVM_ACTOR_CODE_ID);
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

    let result = util::invoke_contract(&rt, &solidity_params);
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

    let result = util::invoke_contract(&rt, &solidity_params);
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

    let rt = util::construct_and_verify(init_code);

    rt.reset();
    rt.expect_validate_caller_any();
    let returned_bytecode_cid: Cid = rt
        .call::<evm::EvmContractActor>(evm::Method::GetBytecode as u64, None)
        .unwrap()
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

    let rt = util::construct_and_verify(init_code);

    rt.reset();
    let params = evm::GetStorageAtParams { storage_key: 0x8965.into() };

    let sender = Address::new_id(0); // zero address because this method is not invokable on-chain
    rt.expect_validate_caller_addr(vec![sender]);
    rt.caller.replace(sender);

    //
    // Get the storage key that was initialized in the init code.
    //
    let value: U256 = rt
        .call::<evm::EvmContractActor>(
            evm::Method::GetStorageAt as u64,
            IpldBlock::serialize_cbor(&params).unwrap(),
        )
        .unwrap()
        .unwrap()
        .deserialize()
        .unwrap();
    rt.verify();
    rt.reset();

    assert_eq!(U256::from(0xfffa), value);

    //
    // Get a storage key that doesn't exist, should default to zero.
    //
    let params = evm::GetStorageAtParams { storage_key: 0xaaaa.into() };

    rt.expect_validate_caller_addr(vec![sender]);
    let value: U256 = rt
        .call::<evm::EvmContractActor>(
            evm::Method::GetStorageAt as u64,
            IpldBlock::serialize_cbor(&params).unwrap(),
        )
        .unwrap()
        .unwrap()
        .deserialize()
        .unwrap();

    assert_eq!(U256::from(0), value);
    rt.verify();
}

#[test]
fn test_push_last_byte() {
    // 60 01 # len
    // 80    # dup len
    // 60 0b # offset 0x0b
    // 60 0  # mem offset 0
    // 39    # codecopy (dstOff, off, len)
    //       # stack = [0x01]
    // 60 0  # mem offset 0
    // f3    # return (offset, size)
    // 7f    # (bytecode)

    // // Inputs[1] { @000A  memory[0x00:0x01] }
    // 0000    60  PUSH1 0x01
    // 0002    80  DUP1
    // 0003    60  PUSH1 0x0b
    // 0005    60  PUSH1 0x00
    // 0007    39  CODECOPY
    // 0008    60  PUSH1 0x00
    // 000A    F3  *RETURN
    // // Stack delta = +0
    // // Outputs[2]
    // // {
    // //     @0007  memory[0x00:0x01] = code[0x0b:0x0c]
    // //     @000A  return memory[0x00:0x01];
    // // }
    // // Block terminates

    // 000B    7F    PUSH32 0x

    // function main() {
    //     memory[0x00:0x01] = code[0x0b:0x0c];
    //     return memory[0x00:0x01];
    // }

    // bytecode where push32 opcode is the last/only byte
    let init_code = hex::decode("600180600b6000396000f37f").unwrap();

    let rt = util::construct_and_verify(init_code);

    util::invoke_contract(&rt, &[]);
}
