mod asm;

use evm::interpreter::address::EthAddress;
use evm::interpreter::instructions::lifecycle::{Create2Params, EamReturn, CREATE2_METHOD_NUM};
use evm::interpreter::U256;
use fil_actor_evm as evm;
use fil_actors_runtime::EAM_ACTOR_ADDR;
use fvm_ipld_encoding::RawBytes;
use fvm_shared::address::Address;
use fvm_shared::econ::TokenAmount;
use fvm_shared::error::ExitCode;

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
push1 0x01   # endowment 1
push1 0x10   # in size (16 bytes)
push2 0x0110 # in offset
push1 0x00   # salt
create2
return
"#;

    asm::new_contract("magic-precompile", init, body).unwrap()
}

#[test]
fn test_create() {
    let contract = magic_precompile_contract();
    let mut rt = util::construct_and_verify(contract);

    rt.add_balance(TokenAmount::from_atto(1));

    // TODO
    let expected_addr = EthAddress([0; 20]);

    let params = Create2Params { code: vec![0; 3], salt: [0; 32] };
    let ret = EamReturn {
        actor_id: 0,
        eth_address: expected_addr,
        robust_address: (&expected_addr).try_into().unwrap(),
    };

    rt.expect_send(
        EAM_ACTOR_ADDR,
        CREATE2_METHOD_NUM,
        RawBytes::serialize(params).unwrap(),
        TokenAmount::from_atto(1),
        RawBytes::serialize(ret).unwrap(),
        ExitCode::OK,
    );

    // invoke contract
    let contract_params = vec![0u8; 32];
    let input_data = RawBytes::from(contract_params);

    let result = util::invoke_contract(&mut rt, input_data);

    // let expected =
    //     hex_literal::hex!("ace8597929092c14bd028ede7b07727875788c7e130278b5afed41940d965aba");
    // assert_eq!(
    //     U256::from_big_endian(&result),
    //     U256::from(expected),
    //     "\n{}\n{}",
    //     hex::encode(&*result),
    //     hex::encode(expected)
    // );
}
