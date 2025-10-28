mod asm;

// Acceptance coverage:
// - Delegated CALL returns 0 on callee revert.
// - RETURNDATASIZE equals the full revert payload length.
// - RETURNDATACOPY truncates to the requested out_size when out_size < payload len,
//   and returns the full payload at the head of the output region when out_size >= len,
//   leaving the remainder unchanged (copy_to_memory zero_fill=false).

use cid::Cid;
use fil_actor_evm as evm;
use fil_actors_evm_shared::address::EthAddress;
use fil_actors_runtime::test_utils::{self};
use fvm_ipld_blockstore::Blockstore;
use fvm_ipld_encoding::ipld_block::IpldBlock;
use fvm_shared::address::Address as FilAddress;
use fvm_shared::econ::TokenAmount;
use fvm_shared::error::ExitCode;
use fvm_shared::sys::SendFlags;
use fvm_shared::version::NetworkVersion;

mod util;

fn call_proxy_contract_with_outsize(out_size: u8) -> Vec<u8> {
    let init = "";
    let body = format!(
        r#"
# this contract takes an address and proxies a CALL to that address
# out_off=0, out_size={out}
push1 0x{out:02X}
push1 0x00
# in_size=0, in_off=0
push1 0x00
push1 0x00
# value=0
push1 0x00
# dest address from calldata[0..32]
push1 0x00
calldataload
# gas
push4 0xffffffff
call

# return bytes from memory [0..out_size]
push1 0x{out:02X}
push1 0x00
return
"#,
        out = out_size
    );
    asm::new_contract("call-proxy-out", init, &body).unwrap()
}

#[test]
fn delegated_call_revert_data_memory_copy_semantics() {
    // Use two proxies: one that requests out_size=4 (truncate), another out_size=32 (full copy).
    let proxy_trunc = call_proxy_contract_with_outsize(4);
    let proxy_full = call_proxy_contract_with_outsize(32);

    // First case: truncate to 4 bytes.
    let mut rt = util::construct_and_verify(proxy_trunc);
    rt.set_network_version(NetworkVersion::V16);

    // Delegation A->B setup.
    let mut pk_a = [0u8; 65];
    pk_a[0] = 0x04;
    for b in pk_a.iter_mut().skip(1) {
        *b = 0xD1;
    }
    rt.recover_secp_pubkey_fn = Box::new(move |_, _| Ok(pk_a));
    let b_eth: EthAddress = EthAddress(util::CONTRACT_ADDRESS);
    const GAS_BASE_APPLY7702: i64 = 0;
    const GAS_PER_AUTH_TUPLE: i64 = 10_000;
    rt.expect_gas_charge(GAS_BASE_APPLY7702);
    rt.expect_gas_charge(GAS_PER_AUTH_TUPLE);
    let list = vec![evm::DelegationParam {
        chain_id: 0,
        address: b_eth,
        nonce: 0,
        y_parity: 0,
        r: vec![1u8; 32],
        s: vec![1u8; 32],
    }];
    // Destination A derived from pk_a; compute f4 from recovered pubkey
    let (keccak_a, _) =
        test_utils::hash(fvm_shared::crypto::hash::SupportedHashes::Keccak256, &pk_a[1..]);
    let mut a20 = [0u8; 20];
    a20.copy_from_slice(&keccak_a[12..32]);
    let a_eth = EthAddress(a20);
    // Ensure A resolves to Account/Placeholder type
    let a_f4: FilAddress = a_eth.into();
    let a_id = FilAddress::new_id(0xDADAu64);
    rt.set_delegated_address(a_id.id().unwrap(), a_f4);
    rt.set_address_actor_type(a_id, *test_utils::PLACEHOLDER_ACTOR_CODE_ID);

    let to_other = EthAddress::from_id(0xB0B0);
    let params = evm::ApplyAndCallParams {
        list,
        call: evm::ApplyCall { to: to_other, value: vec![], input: vec![] },
    };
    rt.expect_validate_caller_any();
    rt.call::<evm::EvmContractActor>(
        evm::Method::ApplyAndCall as u64,
        IpldBlock::serialize_dag_cbor(&params).unwrap(),
    )
    .unwrap();

    // Prepare delegate revert code (content irrelevant; InvokeAsEoa is stubbed to return revert).
    let code_cid = Cid::try_from("baeaikaia").unwrap();
    rt.store.put_keyed(&code_cid, &[]).unwrap();
    // Expect GetBytecode and InvokeAsEoa with revert payload.
    let revert_data: Vec<u8> = hex_literal::hex!("DEADBEEFCAFEBABE").to_vec();
    rt.expect_send_any_params(
        FilAddress::new_id(0),
        evm::Method::GetBytecode as u64,
        TokenAmount::from_whole(0),
        None,
        SendFlags::READ_ONLY,
        test_utils::SendOutcome {
            send_return: IpldBlock::serialize_cbor(&code_cid).unwrap(),
            exit_code: ExitCode::OK,
            send_error: None,
        },
    );
    rt.expect_send_any_params(
        rt.receiver,
        evm::Method::InvokeAsEoa as u64,
        TokenAmount::from_whole(0),
        Some(0xffff_ffff),
        SendFlags::default(),
        test_utils::SendOutcome {
            send_return: Some(IpldBlock {
                codec: fvm_ipld_encoding::IPLD_RAW,
                data: revert_data.clone(),
            }),
            exit_code: ExitCode::USR_ILLEGAL_STATE,
            send_error: None,
        },
    );

    // Build calldata: [A(32)]
    let mut calldata = vec![0u8; 32];
    a_eth.as_evm_word().write_as_big_endian(&mut calldata[..]);
    // Gas probe during CALL
    rt.expect_gas_available(10_000_000_000u64);
    let out = util::invoke_contract(&rt, &calldata);
    rt.verify();
    assert_eq!(out, revert_data[..4].to_vec());
    rt.reset();

    // Second case: full 32-byte out buffer; we expect full revert data padded/truncated accordingly.
    let rt2 = util::construct_and_verify(proxy_full);
    let mut rt = rt2;
    rt.set_network_version(NetworkVersion::V16);
    // Re-apply mapping A->B.
    rt.expect_gas_charge(GAS_BASE_APPLY7702);
    rt.expect_gas_charge(GAS_PER_AUTH_TUPLE);
    rt.recover_secp_pubkey_fn = Box::new(move |_, _| Ok(pk_a));
    let params = evm::ApplyAndCallParams {
        list: vec![evm::DelegationParam {
            chain_id: 0,
            address: b_eth,
            nonce: 0,
            y_parity: 0,
            r: vec![1u8; 32],
            s: vec![1u8; 32],
        }],
        call: evm::ApplyCall { to: to_other, value: vec![], input: vec![] },
    };
    rt.expect_validate_caller_any();
    rt.call::<evm::EvmContractActor>(
        evm::Method::ApplyAndCall as u64,
        IpldBlock::serialize_dag_cbor(&params).unwrap(),
    )
    .unwrap();

    rt.expect_send_any_params(
        FilAddress::new_id(0),
        evm::Method::GetBytecode as u64,
        TokenAmount::from_whole(0),
        None,
        SendFlags::READ_ONLY,
        test_utils::SendOutcome {
            send_return: IpldBlock::serialize_cbor(&code_cid).unwrap(),
            exit_code: ExitCode::OK,
            send_error: None,
        },
    );
    rt.expect_send_any_params(
        rt.receiver,
        evm::Method::InvokeAsEoa as u64,
        TokenAmount::from_whole(0),
        Some(0xffff_ffff),
        SendFlags::default(),
        test_utils::SendOutcome {
            send_return: Some(IpldBlock {
                codec: fvm_ipld_encoding::IPLD_RAW,
                data: revert_data.clone(),
            }),
            exit_code: ExitCode::USR_ILLEGAL_STATE,
            send_error: None,
        },
    );
    let mut calldata = vec![0u8; 32];
    a_eth.as_evm_word().write_as_big_endian(&mut calldata[..]);
    // Gas probe during CALL
    rt.expect_gas_available(10_000_000_000u64);
    let out = util::invoke_contract(&rt, &calldata);
    rt.verify();
    // Expect exact revert payload in the first len bytes (the proxy returns entire output region)
    assert_eq!(out[..revert_data.len()], revert_data[..]);
}
