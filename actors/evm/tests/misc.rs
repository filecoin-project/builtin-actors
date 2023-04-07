mod asm;
mod util;

use cid::Cid;
use fil_actors_evm_shared::address::EthAddress;
use fil_actors_evm_shared::uints::U256;
use fvm_ipld_encoding::DAG_CBOR;
use fvm_shared::chainid::ChainID;
use fvm_shared::{address::Address, econ::TokenAmount};
use multihash::Multihash;

#[test]
fn test_timestamp() {
    let contract = asm::new_contract(
        "timestamp",
        "",
        r#"
timestamp
push1 0x00
mstore
push1 0x20
push1 0x00
return
"#,
    )
    .unwrap();

    let mut rt = util::construct_and_verify(contract);
    rt.tipset_timestamp = 123;
    let result = util::invoke_contract(&rt, &[]);
    assert_eq!(U256::from_big_endian(&result), U256::from(123));
}

#[test]
fn test_blockhash() {
    let contract = asm::new_contract(
        "blockhash",
        "",
        r#"
push2 0xffff
blockhash
push1 0x00
mstore
push1 0x20
push1 0x00
return
"#,
    )
    .unwrap();

    let mut rt = util::construct_and_verify(contract);

    rt.tipset_cids = (0..(0xffff + 512))
        .map(|i| {
            Cid::new_v1(DAG_CBOR, Multihash::wrap(0, format!("block-{:026}", i).as_ref()).unwrap())
        })
        .collect();

    rt.epoch.replace(0xffff + 2);
    let result = util::invoke_contract(&rt, &[]);
    assert_eq!(
        String::from_utf8_lossy(&result.to_vec()),
        String::from_utf8_lossy(rt.tipset_cids[0xffff].hash().digest())
    );

    rt.epoch.replace(0xffff + 256);
    let result = util::invoke_contract(&rt, &[]);
    assert_eq!(
        String::from_utf8_lossy(&result.to_vec()),
        String::from_utf8_lossy(rt.tipset_cids[0xffff].hash().digest())
    );

    rt.epoch.replace(0xffff);
    let result = util::invoke_contract(&rt, &[]);
    assert_eq!(&result, &[0u8; 32]);

    rt.epoch.replace(0xffff - 1);
    let result = util::invoke_contract(&rt, &[]);
    assert_eq!(&result, &[0u8; 32]);

    rt.epoch.replace(0xffff + 257);
    let result = util::invoke_contract(&rt, &[]);
    assert_eq!(&result, &[0u8; 32]);
}

#[test]
fn test_chainid() {
    let contract = asm::new_contract(
        "chainid",
        "",
        r#"
chainid
push1 0x00
mstore
push1 0x20
push1 0x00
return
"#,
    )
    .unwrap();

    let mut rt = util::construct_and_verify(contract);
    rt.chain_id = ChainID::from(1989);
    let result = util::invoke_contract(&rt, &[]);
    assert_eq!(U256::from_big_endian(&result), U256::from(1989));
}

#[test]
fn test_gas_limit() {
    let contract = asm::new_contract(
        "gaslimit",
        "",
        r#"
gaslimit
push1 0x00
mstore
push1 0x20
push1 0x00
return
"#,
    )
    .unwrap();

    let rt = util::construct_and_verify(contract);
    let result = util::invoke_contract(&rt, &[]);
    assert_eq!(U256::from_big_endian(&result), U256::from(10_000_000_000u64));
}

#[test]
fn test_gas_price() {
    let contract = asm::new_contract(
        "timestamp",
        "",
        r#"
gasprice
push1 0x00
mstore
push1 0x20
push1 0x00
return
"#,
    )
    .unwrap();

    let mut rt = util::construct_and_verify(contract);
    rt.base_fee.replace(TokenAmount::from_atto(123));
    rt.gas_premium = TokenAmount::from_atto(345);
    let result = util::invoke_contract(&rt, &[]);
    assert_eq!(U256::from_big_endian(&result), U256::from(123 + 345));
}

#[test]
fn test_balance() {
    let contract = asm::new_contract(
        "balance",
        "",
        r#"
push1 0x20
push1 0x00
push1 0x00
calldatacopy
push1 0x00
mload
balance
push1 0x00
mstore
push1 0x20
push1 0x00
return
"#,
    )
    .unwrap();

    let mut rt = util::construct_and_verify(contract);
    rt.actor_balances.insert(100, TokenAmount::from_atto(123));
    let mut input_data = vec![0u8; 32];
    input_data[12] = 0xff;
    input_data[31] = 0x64;
    let result = util::invoke_contract(&rt, &input_data);
    assert_eq!(U256::from_big_endian(&result), U256::from(123));
}

#[test]
fn test_balance_bogus() {
    let contract = asm::new_contract(
        "balance",
        "",
        r#"
push1 0x20
push1 0x00
push1 0x00
calldatacopy
push1 0x00
mload
balance
push1 0x00
mstore
push1 0x20
push1 0x00
return
"#,
    )
    .unwrap();

    let rt = util::construct_and_verify(contract);
    let mut input_data = vec![0u8; 32];
    input_data[31] = 123;
    let result = util::invoke_contract(&rt, &input_data);
    assert_eq!(U256::from_big_endian(&result), U256::from(0));
}

#[test]
fn test_balance0() {
    let contract = asm::new_contract(
        "balance",
        "",
        r#"
push1 0x00
balance
push1 0x00
mstore
push1 0x20
push1 0x00
return
"#,
    )
    .unwrap();

    let rt = util::construct_and_verify(contract);
    let result = util::invoke_contract(&rt, &[]);
    assert_eq!(U256::from_big_endian(&result), U256::from(0));
}

#[test]
fn test_gas() {
    let contract = asm::new_contract(
        "gas",
        "",
        r#"
gas
push1 0x00
mstore
push1 0x20
push1 0x00
return
"#,
    )
    .unwrap();

    let rt = util::construct_and_verify(contract);
    rt.expect_gas_available(123);
    let result = util::invoke_contract(&rt, &[]);
    assert_eq!(U256::from_big_endian(&result), U256::from(123));
}

#[test]
fn test_address() {
    let contract = asm::new_contract(
        "gas",
        "",
        r#"
push1 0x00
address
push1 0x00
mstore
push1 0x20
push1 0x00
return
"#,
    )
    .unwrap();

    let rt = util::construct_and_verify(contract);
    let result = util::invoke_contract(&rt, &[]);
    let eth_address = &result[12..];
    // Make sure we get an actual eth address, not an embedded ID address.
    assert_eq!(&eth_address, &util::CONTRACT_ADDRESS);
}

#[test]
fn test_caller_id() {
    let contract = asm::new_contract(
        "gas",
        "",
        r#"
push1 0x00
caller
push1 0x00
mstore
push1 0x20
push1 0x00
return
"#,
    )
    .unwrap();

    let rt = util::construct_and_verify(contract);
    let result = util::invoke_contract(&rt, &[]);
    let eth_address = &result[12..];
    // The caller's address should be the init actor in this case.
    assert_eq!(&eth_address, &EthAddress::from_id(1).0);
}

#[test]
fn test_caller_eth() {
    let contract = asm::new_contract(
        "gas",
        "",
        r#"
push1 0x00
caller
push1 0x00
mstore
push1 0x20
push1 0x00
return
"#,
    )
    .unwrap();

    let rt = util::construct_and_verify(contract);
    // set the _id_ address here (ensures we resolve it correctly internally).
    rt.caller.replace(Address::new_id(0));
    let result = util::invoke_contract(&rt, &[]);
    let eth_address = &result[12..];
    // Make sure we prefer the eth address, if we have one.
    assert_eq!(eth_address, util::CONTRACT_ADDRESS);
}

#[test]
fn test_origin_id() {
    let contract = asm::new_contract(
        "gas",
        "",
        r#"
push1 0x00
origin
push1 0x00
mstore
push1 0x20
push1 0x00
return
"#,
    )
    .unwrap();

    let rt = util::construct_and_verify(contract);
    rt.origin.replace(Address::new_id(10));
    let result = util::invoke_contract(&rt, &[]);
    let eth_address = &result[12..];
    // Make sure we prefer the eth address, if we have one.
    assert_eq!(eth_address, &EthAddress::from_id(10).0);
}

#[test]
fn test_origin_eth() {
    let contract = asm::new_contract(
        "gas",
        "",
        r#"
push1 0x00
origin
push1 0x00
mstore
push1 0x20
push1 0x00
return
"#,
    )
    .unwrap();

    let rt = util::construct_and_verify(contract);
    let result = util::invoke_contract(&rt, &[]);
    let eth_address = &result[12..];
    // Make sure we prefer the eth address, if we have one.
    assert_eq!(eth_address, util::CONTRACT_ADDRESS);
}
