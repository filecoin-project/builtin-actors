use fil_actor_evm::interpreter::opcodes;
use fil_actors_evm_shared::uints::U256;

mod util;

fn initcode_for_runtime(runtime: &[u8]) -> Vec<u8> {
    let len: u16 = runtime.len().try_into().expect("runtime too large");

    // initcode:
    // PUSH2 len
    // PUSH2 offset
    // PUSH1 0x00
    // CODECOPY
    // PUSH2 len
    // PUSH1 0x00
    // RETURN
    // <runtime bytes>
    let init_len: u16 = 15;
    let offset = init_len;

    let [len_hi, len_lo] = len.to_be_bytes();
    let [off_hi, off_lo] = offset.to_be_bytes();

    let mut code = vec![
        opcodes::PUSH2,
        len_hi,
        len_lo,
        opcodes::PUSH2,
        off_hi,
        off_lo,
        opcodes::PUSH1,
        0x00,
        opcodes::CODECOPY,
        opcodes::PUSH2,
        len_hi,
        len_lo,
        opcodes::PUSH1,
        0x00,
        opcodes::RETURN,
    ];
    code.extend_from_slice(runtime);
    code
}

fn clz_runtime() -> Vec<u8> {
    // Reads a 32-byte word from calldata[0..32], computes CLZ, and returns the 32-byte result.
    vec![
        opcodes::PUSH1,
        0x00,
        opcodes::CALLDATALOAD,
        opcodes::CLZ,
        opcodes::PUSH1,
        0x00,
        opcodes::MSTORE,
        opcodes::PUSH1,
        0x20,
        opcodes::PUSH1,
        0x00,
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

    let vectors = [
        (U256::ZERO, U256::from(256)),
        (U256::ONE << 255, U256::ZERO),
        (U256::MAX, U256::ZERO),
        (U256::ONE << 254, U256::ONE),
        ((U256::ONE << 255) - U256::ONE, U256::ONE),
        (U256::ONE, U256::from(255)),
    ];

    for (input, expected) in vectors {
        let ret = util::invoke_contract(&rt, &u256_be_bytes(input));
        assert_eq!(ret.len(), 32);
        assert_eq!(ret.as_slice(), &u256_be_bytes(expected));
    }
}

