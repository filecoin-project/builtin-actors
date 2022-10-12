mod asm;

use evm::interpreter::U256;
use fil_actor_evm as evm;
use fvm_ipld_encoding::RawBytes;

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
fn test_precompile_hash() {
    let contract = magic_precompile_contract();
    let mut rt = util::construct_and_verify(contract);

    // invoke contract
    let contract_params = vec![0u8; 32];
    let input_data = RawBytes::from(contract_params);

    let result = util::invoke_contract(&mut rt, input_data);
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
