use fil_actor_evm as evm;
use fil_actors_evm_shared::address::EthAddress;
use fvm_ipld_encoding::ipld_block::IpldBlock;
use fvm_shared::address::Address as FilAddress;
use fvm_shared::econ::TokenAmount;
use fvm_shared::error::ExitCode;
use fvm_shared::sys::SendFlags;
use fvm_shared::version::NetworkVersion;

mod util;
mod asm;

fn call_proxy_contract() -> Vec<u8> {
    let init = "";
    let body = r#"
push1 0x20
calldatasize
sub
push1 0x20
push1 0x00
calldatacopy
push2 0x00
push1 0x00
push1 0x20
calldatasize
sub
push1 0x00
push1 0x00
push1 0x00
calldataload
push4 0xffffffff
call
returndatasize
push1 0x00
push1 0x00
returndatacopy
returndatasize
push1 0x00
return
"#;
    asm::new_contract("call-proxy-absent", init, body).unwrap()
}

// Post-activation: When Delegator returns None (no mapping), EVM should fall back to direct
// InvokeContract on the EOA f4 address without emitting the EIP7702 event.
#[test]
fn call_to_eoa_no_mapping_direct_call() {
    let initcode = call_proxy_contract();
    let rt = util::construct_and_verify(initcode);
    rt.set_network_version(NetworkVersion::V16);

    // Destination EOA
    let authority = EthAddress(hex_literal::hex!("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"));
    let authority_word = authority.as_evm_word();
    let authority_f4: FilAddress = authority.into();

    // Expect Delegator LookupDelegate(authority) -> None
    #[derive(fvm_ipld_encoding::serde::Serialize, fvm_ipld_encoding::serde::Deserialize)]
    struct LookupDelegateParams { authority: EthAddress }
    #[derive(fvm_ipld_encoding::serde::Serialize, fvm_ipld_encoding::serde::Deserialize)]
    struct LookupDelegateReturn { delegate: Option<EthAddress> }
    rt.expect_send(
        fil_actors_runtime::DELEGATOR_ACTOR_ADDR,
        frc42_dispatch::method_hash!("LookupDelegate"),
        IpldBlock::serialize_cbor(&LookupDelegateParams { authority }).unwrap(),
        TokenAmount::from_whole(0),
        None,
        SendFlags::READ_ONLY,
        IpldBlock::serialize_cbor(&LookupDelegateReturn { delegate: None }).unwrap(),
        ExitCode::OK,
        None,
    );

    // Gas query used to compute call gas limit.
    rt.expect_gas_available(10_000_000_000u64);

    // Expect: direct InvokeContract to EOA (no event expected).
    // We'll mock some return data and assert passthrough.
    let expected_output = vec![0x11, 0x22];
    rt.expect_send(
        authority_f4,
        evm::Method::InvokeContract as u64,
        None,
        TokenAmount::from_whole(0),
        Some(0xffff_ffff),
        SendFlags::empty(),
        Some(IpldBlock { codec: fvm_shared::IPLD_RAW, data: expected_output.clone() }),
        ExitCode::OK,
        None,
    );

    // Build call params: [dest(32b)]
    let mut call_params = vec![0u8; 32];
    authority_word.write_as_big_endian(&mut call_params[..]);

    let result = util::invoke_contract(&rt, &call_params);
    assert_eq!(result, expected_output);
}
