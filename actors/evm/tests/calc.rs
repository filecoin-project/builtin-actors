mod asm;

use evm::interpreter::U256;
use fil_actor_evm as evm;
use fvm_ipld_encoding::RawBytes;

mod util;

#[allow(dead_code)]
pub fn magic_calc_contract() -> Vec<u8> {
    let init = r#"
push1 0x42  # magic value
push1 0x00  # key of magic value
sstore
"#;
    let body = r#"
# method dispatch:
# - 0x00000000 -> magic value
# - 0x00000001 -> ADD arg, magic value
# - 0x00000002 -> MUL arg, magic value

%dispatch_begin()
%dispatch(0x00, get_magic)
%dispatch(0x01, add_magic)
%dispatch(0x02, mul_magic)
%dispatch_end()

#### method implementation
get_magic:
jumpdest
push1 0x20 # length of return data
push1 0x00 # key of magic
sload
push1 0x00 # return memory offset
mstore
push1 0x00
return

add_magic:
jumpdest
push1 0x20   # length of return data
push1 0x04
calldataload # arg1
push1 0x00   # key of magic
sload
add
push1 0x00   # return memory offset
mstore
push1 0x00
return

mul_magic:
jumpdest
push1 0x20   # length of return dataa
push1 0x04
calldataload # arg1
push1 0x00   # key of magic
sload
mul
push1 0x00   # return memory offset
mstore
push1 0x00
return

"#;

    asm::new_contract("magic-calc", init, body).unwrap()
}

#[test]
fn test_magic_calc() {
    let contract = magic_calc_contract();

    let mut rt = util::construct_and_verify(contract);

    // invoke contract -- get_magic
    let contract_params = vec![0u8; 32];
    let input_data = RawBytes::from(contract_params);

    let result = util::invoke_contract(&mut rt, input_data);
    assert_eq!(U256::from_big_endian(&result), U256::from(0x42));

    // invoke contract -- add_magic
    let mut contract_params = vec![0u8; 36];
    contract_params[3] = 0x01;
    contract_params[35] = 0x01;
    let input_data = RawBytes::from(contract_params);

    let result = util::invoke_contract(&mut rt, input_data);
    assert_eq!(U256::from_big_endian(&result), U256::from(0x43));

    // invoke contract -- mul_magic
    let mut contract_params = vec![0u8; 36];
    contract_params[3] = 0x02;
    contract_params[35] = 0x02;
    let input_data = RawBytes::from(contract_params);

    let result = util::invoke_contract(&mut rt, input_data);
    assert_eq!(U256::from_big_endian(&result), U256::from(0x84));
}
