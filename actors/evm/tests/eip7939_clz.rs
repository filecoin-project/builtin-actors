use fil_actor_evm::interpreter::opcodes;
use fil_actors_evm_shared::uints::U256;

mod test_vectors;
mod util;

fn initcode_for_runtime(runtime: &[u8]) -> Vec<u8> {
    // Universal runtime constructor:
    // https://github.com/wjmelements/evm/pull/87
    const CONSTRUCTOR: [u8; 11] = [
        opcodes::PUSH1,
        0x0b,
        opcodes::CODESIZE,
        opcodes::SUB,
        opcodes::DUP1,
        opcodes::PUSH1,
        0x0b,
        opcodes::RETURNDATASIZE,
        opcodes::CODECOPY,
        opcodes::RETURNDATASIZE,
        opcodes::RETURN,
    ];

    let mut code = Vec::with_capacity(CONSTRUCTOR.len() + runtime.len());
    code.extend_from_slice(&CONSTRUCTOR);
    code.extend_from_slice(runtime);
    code
}

fn clz_runtime() -> Vec<u8> {
    // Reads a 32-byte word from calldata[0..32], computes CLZ, and returns the 32-byte result.
    vec![
        opcodes::PUSH0,
        opcodes::CALLDATALOAD,
        opcodes::CLZ,
        opcodes::PUSH0,
        opcodes::MSTORE,
        opcodes::MSIZE,
        opcodes::PUSH0,
        opcodes::RETURN,
    ]
}

fn u256_be_bytes(value: U256) -> [u8; 32] {
    let mut out = [0u8; 32];
    value.write_as_big_endian(&mut out);
    out
}

#[test]
fn eip7939_clz_vectors_end_to_end() {
    let initcode = initcode_for_runtime(&clz_runtime());
    let rt = util::construct_and_verify(initcode);

    for (input, expected) in test_vectors::clz_eip7939_test_vectors() {
        let ret = util::invoke_contract(&rt, &u256_be_bytes(input));
        assert_eq!(ret.len(), 32);
        assert_eq!(ret.as_slice(), &u256_be_bytes(expected));
    }
}
