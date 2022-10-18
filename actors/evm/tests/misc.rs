mod asm;
mod util;

use cid::Cid;
use evm::interpreter::U256;
use fil_actor_evm as evm;
use fvm_shared::econ::TokenAmount;

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
    let result = util::invoke_contract(&mut rt, &[]);
    assert_eq!(U256::from_big_endian(&result), U256::from(123));
}

#[test]
fn test_blockhash() {
    let contract = asm::new_contract(
        "blockhash",
        "",
        r#"
push1 0x00
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

    let test_cid =
        Cid::try_from("bafy2bzacecu7n7wbtogznrtuuvf73dsz7wasgyneqasksdblxupnyovmtwxxu").unwrap();
    rt.tipset_cids = vec![test_cid];
    let result = util::invoke_contract(&mut rt, &[]);
    assert_eq!(result.to_vec(), test_cid.hash().digest());
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
    let result = util::invoke_contract(&mut rt, &[]);
    assert_eq!(U256::from_big_endian(&result), U256::from(31415926));
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

    let mut rt = util::construct_and_verify(contract);
    let result = util::invoke_contract(&mut rt, &[]);
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
    rt.base_fee = TokenAmount::from_atto(123);
    rt.gas_premium = TokenAmount::from_atto(345);
    let result = util::invoke_contract(&mut rt, &[]);
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
    let result = util::invoke_contract(&mut rt, &input_data);
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

    let mut rt = util::construct_and_verify(contract);
    let mut input_data = vec![0u8; 32];
    input_data[31] = 123;
    let result = util::invoke_contract(&mut rt, &input_data);
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

    let mut rt = util::construct_and_verify(contract);
    let result = util::invoke_contract(&mut rt, &[]);
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

    let mut rt = util::construct_and_verify(contract);
    rt.expect_gas_available(123);
    let result = util::invoke_contract(&mut rt, &[]);
    assert_eq!(U256::from_big_endian(&result), U256::from(123));
}
