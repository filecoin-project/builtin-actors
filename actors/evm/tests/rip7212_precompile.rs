mod asm;
mod util;

use fvm_shared::{METHOD_SEND, address::Address as FILAddress, econ::TokenAmount, error::ExitCode};
use serde::Deserialize;

const TESTDATA_PATH: &str =
    concat!(env!("CARGO_MANIFEST_DIR"), "/precompile-testdata/eip7951_p256verify.json");

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "PascalCase")]
struct Eip7951TestVector {
    name: String,
    input: String,
    expected: String,
    gas: u64,
    #[serde(default)]
    no_benchmark: bool,
}

fn load_p256_vectors() -> Vec<Eip7951TestVector> {
    let testdata =
        std::fs::read_to_string(TESTDATA_PATH).expect("failed to read EIP-7951 test vector file");
    serde_json::from_str(&testdata).expect("failed to parse EIP-7951 test vectors")
}

fn sample_success_vector() -> Eip7951TestVector {
    load_p256_vectors()
        .into_iter()
        .find(|vector| !vector.no_benchmark && !vector.expected.is_empty())
        .expect("expected at least one successful EIP-7951 test vector")
}

fn sample_failure_vector() -> Eip7951TestVector {
    load_p256_vectors()
        .into_iter()
        .find(|vector| !vector.no_benchmark && vector.expected.is_empty())
        .expect("expected at least one failing EIP-7951 test vector")
}

fn decode_hex(value: &str, kind: &str, vector: &Eip7951TestVector) -> Vec<u8> {
    hex::decode(value).unwrap_or_else(|error| {
        panic!("failed to decode {kind} for {} (gas={}): {error}", vector.name, vector.gas)
    })
}

fn p256_verify_contract_call() -> Vec<u8> {
    // Call 0x0100 precompile with calldata as input (exact 160 bytes)
    let init = "";
    let body = r#"

calldatasize
push1 0x00
push1 0x00
calldatacopy

# out size
push1 0x20
# out off
push2 0xA000

# in size (160)
push1 0xA0
# in off
push1 0x00

# value (0)
push1 0x00

# dst (0x0100)
push20 0x0000000000000000000000000000000000000100

# gas
push1 0x00

call

# write exit code memory
push1 0x00 # offset
mstore8

returndatasize
push1 0x00 # input offset
push1 0x01 # dest offset
returndatacopy

returndatasize
push1 0x01
add
push1 0x00
return
"#;
    asm::new_contract("rip7212-precompile-caller", init, body).unwrap()
}

#[test]
fn rip7212_call_success_vector() {
    let rt = util::construct_and_verify(p256_verify_contract_call());

    let vector = sample_success_vector();
    let input = decode_hex(&vector.input, "input", &vector);
    let expected = decode_hex(&vector.expected, "expected", &vector);

    assert_eq!(expected.len(), 32, "success vectors must return 32-byte output");
    assert_eq!(expected[31], 1, "success vectors must end with 0x01");

    let result = util::invoke_contract(&rt, &input);
    assert_eq!(result[0], util::PrecompileExit::Success as u8);
    assert_eq!(&result[1..], expected.as_slice());
}

#[test]
fn rip7212_call_failure_vector_returns_empty() {
    let rt = util::construct_and_verify(p256_verify_contract_call());

    let vector = sample_failure_vector();
    let input = decode_hex(&vector.input, "input", &vector);
    let expected = decode_hex(&vector.expected, "expected", &vector);

    assert!(expected.is_empty(), "failure vectors must expect empty output");

    let result = util::invoke_contract(&rt, &input);
    assert_eq!(result[0], util::PrecompileExit::Success as u8);
    assert_eq!(&result[1..], expected.as_slice());
}

#[test]
fn rip7212_invalid_input_returns_empty() {
    let rt = util::construct_and_verify(p256_verify_contract_call());
    let input = vec![0u8; 10];
    let result = util::invoke_contract(&rt, &input);
    assert_eq!(result[0], util::PrecompileExit::Success as u8);
    assert!(result[1..].is_empty());
}

fn p256_verify_contract_call_value() -> Vec<u8> {
    let init = "";
    let body = r#"

calldatasize
push1 0x00
push1 0x00
calldatacopy

# out size
push1 0x20
# out off
push2 0xA000

# in size (160)
push1 0xA0
# in off
push1 0x00

# value (1 atto)
push1 0x01

# dst (0x0100)
push20 0x0000000000000000000000000000000000000100

# gas
push1 0x00

call

# write exit code memory
push1 0x00 # offset
mstore8

returndatasize
push1 0x00 # input offset
push1 0x01 # dest offset
returndatacopy

returndatasize
push1 0x01
add
push1 0x00
return
"#;
    asm::new_contract("rip7212-precompile-caller-value", init, body).unwrap()
}

#[test]
fn rip7212_call_with_value_transfers_on_success() {
    let rt = util::construct_and_verify(p256_verify_contract_call_value());
    rt.set_balance(TokenAmount::from_atto(100));

    let vector = sample_success_vector();
    let input = decode_hex(&vector.input, "input", &vector);
    let expected = decode_hex(&vector.expected, "expected", &vector);

    let addr = fil_actors_evm_shared::address::EthAddress(hex_literal::hex!(
        "0000000000000000000000000000000000000100"
    ));
    let fil_addr =
        FILAddress::new_delegated(fil_actors_runtime::EAM_ACTOR_ID, addr.as_ref()).unwrap();
    rt.expect_send_simple(
        fil_addr,
        METHOD_SEND,
        None,
        TokenAmount::from_atto(1),
        None,
        ExitCode::OK,
    );

    let result = util::invoke_contract(&rt, &input);
    assert_eq!(result[0], util::PrecompileExit::Success as u8);
    assert_eq!(&result[1..], expected.as_slice());
}
