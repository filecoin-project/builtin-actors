use fil_actor_evm as evm;
use fvm_ipld_encoding::{BytesSer, RawBytes};

mod asm;
mod util;

#[test]
fn test_revert() {
    let contract = asm::new_contract(
        "naked-revert",
        "",
        r#"
%push(0xdeadbeef)
push1 0x00
mstore
push1 0x04
push1 0x1c # skip top 28 bytes
revert
"#,
    )
    .unwrap();

    let rt = util::construct_and_verify(contract);
    rt.expect_validate_caller_any();

    let result = rt.call::<evm::EvmContractActor>(evm::Method::InvokeContract as u64, None);
    assert!(result.is_err());
    let mut e = result.unwrap_err();
    assert_eq!(e.exit_code(), evm::EVM_CONTRACT_REVERTED);
    assert_eq!(
        e.take_data().unwrap().data,
        RawBytes::serialize(BytesSer(&[0xde, 0xad, 0xbe, 0xef])).unwrap().bytes()
    );
}
