mod asm;
mod util;

use fvm_shared::{METHOD_SEND, address::Address as FILAddress, econ::TokenAmount, error::ExitCode};

use p256::ecdsa::signature::hazmat::PrehashSigner;
use p256::ecdsa::{Signature, SigningKey, VerifyingKey};
use rand::rngs::StdRng;
use rand::{RngCore, SeedableRng};

fn p256_input() -> Vec<u8> {
    let mut rng = StdRng::seed_from_u64(7212);
    let sk = SigningKey::random(&mut rng);
    let vk = VerifyingKey::from(&sk);

    let mut hash = [0u8; 32];
    rng.fill_bytes(&mut hash);
    let sig: Signature = PrehashSigner::sign_prehash(&sk, &hash).unwrap();

    let pk = vk.to_encoded_point(false);
    let (x, y) = (pk.x().unwrap(), pk.y().unwrap());

    let mut input = Vec::with_capacity(160);
    input.extend_from_slice(&hash);
    input.extend_from_slice(&sig.r().to_bytes());
    input.extend_from_slice(&sig.s().to_bytes());
    input.extend_from_slice(x);
    input.extend_from_slice(y);
    assert_eq!(input.len(), 160);
    input
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
fn rip7212_call_success() {
    let rt = util::construct_and_verify(p256_verify_contract_call());

    let input = p256_input();
    let result = util::invoke_contract(&rt, &input);
    let mut expected = [0u8; 32];
    expected[31] = 1;
    assert_eq!(result[0], util::PrecompileExit::Success as u8);
    assert_eq!(&result[1..], &expected);
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

    let input = p256_input();
    let mut expected = [0u8; 32];
    expected[31] = 1;

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
    assert_eq!(&result[1..], &expected);
}
