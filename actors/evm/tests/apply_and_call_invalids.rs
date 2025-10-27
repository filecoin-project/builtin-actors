use cid::Cid;
use fil_actor_evm as evm;
use fil_actors_evm_shared::address::EthAddress;
use fil_actors_runtime::test_utils::MockRuntime;
use fvm_ipld_encoding::ipld_block::IpldBlock;
use fvm_shared::econ::TokenAmount;
use fvm_shared::error::ExitCode;
use fvm_shared::sys::SendFlags;

mod util;

#[test]
fn apply_and_call_rejects_invalid_chain_id() {
    let rt = util::construct_and_verify(vec![]);
    const GAS_BASE_APPLY7702: i64 = 0;
    const GAS_PER_AUTH_TUPLE: i64 = 10_000;
    rt.expect_gas_charge(GAS_BASE_APPLY7702);
    rt.expect_gas_charge(GAS_PER_AUTH_TUPLE);

    let authority = EthAddress(hex_literal::hex!("00112233445566778899aabbccddeeff00112233"));
    // chain_id 999 should mismatch most default test runtimes.
    let list = vec![evm::DelegationParam {
        chain_id: 999,
        address: authority,
        nonce: 0,
        y_parity: 0,
        r: vec![1u8; 32],
        s: vec![1u8; 32],
    }];
    let params = evm::ApplyAndCallParams {
        list,
        call: evm::ApplyCall { to: authority, value: vec![0u8], input: vec![] },
    };
    rt.expect_validate_caller_any();
    let res = rt.call::<evm::EvmContractActor>(
        evm::Method::ApplyAndCall as u64,
        IpldBlock::serialize_dag_cbor(&params).unwrap(),
    );
    assert!(res.is_err());
    assert_eq!(res.err().unwrap().exit_code(), ExitCode::USR_ILLEGAL_ARGUMENT);
}

#[test]
fn apply_and_call_rejects_zero_r_or_s() {
    let rt = util::construct_and_verify(vec![]);
    const GAS_BASE_APPLY7702: i64 = 0;
    const GAS_PER_AUTH_TUPLE: i64 = 10_000;
    rt.expect_gas_charge(GAS_BASE_APPLY7702);
    rt.expect_gas_charge(GAS_PER_AUTH_TUPLE);

    let authority = EthAddress(hex_literal::hex!("00112233445566778899aabbccddeeff00112233"));
    let mut zeros = vec![0u8; 32];
    let list = vec![evm::DelegationParam {
        chain_id: 0,
        address: authority,
        nonce: 0,
        y_parity: 0,
        r: zeros.clone(),
        s: vec![1u8; 32],
    }];
    let params = evm::ApplyAndCallParams {
        list,
        call: evm::ApplyCall { to: authority, value: vec![], input: vec![] },
    };
    rt.expect_validate_caller_any();
    let res = rt.call::<evm::EvmContractActor>(
        evm::Method::ApplyAndCall as u64,
        IpldBlock::serialize_dag_cbor(&params).unwrap(),
    );
    assert!(res.is_err());
    assert_eq!(res.err().unwrap().exit_code(), ExitCode::USR_ILLEGAL_ARGUMENT);
}

#[test]
fn apply_and_call_rejects_high_s() {
    let rt = util::construct_and_verify(vec![]);
    const GAS_BASE_APPLY7702: i64 = 0;
    const GAS_PER_AUTH_TUPLE: i64 = 10_000;
    rt.expect_gas_charge(GAS_BASE_APPLY7702);
    rt.expect_gas_charge(GAS_PER_AUTH_TUPLE);

    let authority = EthAddress(hex_literal::hex!("00112233445566778899aabbccddeeff00112233"));
    let mut high_s = vec![0xffu8; 32];
    let list = vec![evm::DelegationParam {
        chain_id: 0,
        address: authority,
        nonce: 0,
        y_parity: 0,
        r: vec![1u8; 32],
        s: high_s.clone(),
    }];
    let params = evm::ApplyAndCallParams {
        list,
        call: evm::ApplyCall { to: authority, value: vec![], input: vec![] },
    };
    rt.expect_validate_caller_any();
    let res = rt.call::<evm::EvmContractActor>(
        evm::Method::ApplyAndCall as u64,
        IpldBlock::serialize_dag_cbor(&params).unwrap(),
    );
    assert!(res.is_err());
    assert_eq!(res.err().unwrap().exit_code(), ExitCode::USR_ILLEGAL_ARGUMENT);
}

#[test]
fn apply_and_call_rejects_invalid_authorizations() {
    let rt = util::construct_and_verify(vec![]);

    // Intrinsic gas expected: base + per-tuple (placeholders).
    const GAS_BASE_APPLY7702: i64 = 0;
    const GAS_PER_AUTH_TUPLE: i64 = 10_000;
    rt.expect_gas_charge(GAS_BASE_APPLY7702);
    rt.expect_gas_charge(GAS_PER_AUTH_TUPLE);

    // Build ApplyAndCall with a single tuple (invalid y_parity; validator should reject).
    let authority = EthAddress(hex_literal::hex!("00112233445566778899aabbccddeeff00112233"));
    let list = vec![evm::DelegationParam {
        chain_id: 0,
        address: authority,
        nonce: 0,
        y_parity: 2,
        r: vec![0u8; 32],
        s: vec![0u8; 32],
    }];
    let params = evm::ApplyAndCallParams {
        list: list.clone(),
        call: evm::ApplyCall { to: authority, value: vec![0u8], input: vec![] },
    };

    // Call ApplyAndCall; expect error with same exit code propagated.
    rt.expect_validate_caller_any();
    let res = rt.call::<evm::EvmContractActor>(
        evm::Method::ApplyAndCall as u64,
        IpldBlock::serialize_dag_cbor(&params).unwrap(),
    );
    assert!(res.is_err());
    let err = res.err().unwrap();
    assert_eq!(err.exit_code(), ExitCode::USR_ILLEGAL_ARGUMENT);
}

#[test]
fn apply_and_call_propagates_outer_call_failure() {
    let rt = util::construct_and_verify(vec![]);

    // Intrinsic gas expected: base + per-tuple (placeholders).
    const GAS_BASE_APPLY7702: i64 = 0;
    const GAS_PER_AUTH_TUPLE: i64 = 10_000;
    rt.expect_gas_charge(GAS_BASE_APPLY7702);
    rt.expect_gas_charge(GAS_PER_AUTH_TUPLE);

    // Build ApplyAndCall with valid-looking tuple and EVM destination.
    let dst = EthAddress(hex_literal::hex!("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"));
    let list = vec![evm::DelegationParam {
        chain_id: 0,
        address: dst,
        nonce: 0,
        y_parity: 0,
        r: vec![1u8; 32],
        s: vec![1u8; 32],
    }];
    let params = evm::ApplyAndCallParams {
        list: list.clone(),
        call: evm::ApplyCall { to: dst, value: vec![0u8], input: vec![0x01] },
    };

    // Destination is EVM contract: expect InvokeContract to fail.
    use fil_actors_runtime::test_utils::EVM_ACTOR_CODE_ID;
    use fvm_shared::address::Address as FilAddress;
    let dst_f4: FilAddress = dst.into();
    let dst_id = FilAddress::new_id(0x333u64);
    rt.set_delegated_address(dst_id.id().unwrap(), dst_f4);
    rt.actor_code_cids.borrow_mut().insert(dst_id, *EVM_ACTOR_CODE_ID);

    rt.expect_send(
        dst_id,
        evm::Method::InvokeContract as u64,
        Some(IpldBlock { codec: fvm_ipld_encoding::IPLD_RAW, data: vec![0x01] }),
        TokenAmount::from_whole(0),
        None,
        SendFlags::empty(),
        None,
        ExitCode::USR_ILLEGAL_STATE, // simulate a failure
        None,
    );

    // Call ApplyAndCall; expect OK (status embedded).
    rt.expect_validate_caller_any();
    let res = rt.call::<evm::EvmContractActor>(
        evm::Method::ApplyAndCall as u64,
        IpldBlock::serialize_dag_cbor(&params).unwrap(),
    );
    assert!(res.is_ok());
}

#[test]
fn apply_and_call_rejects_bad_r_s_lengths() {
    let rt = util::construct_and_verify(vec![]);

    const GAS_BASE_APPLY7702: i64 = 0;
    const GAS_PER_AUTH_TUPLE: i64 = 10_000;
    rt.expect_gas_charge(GAS_BASE_APPLY7702);
    rt.expect_gas_charge(GAS_PER_AUTH_TUPLE);

    let authority = EthAddress(hex_literal::hex!("00112233445566778899aabbccddeeff00112233"));
    // r too short, s too long
    let list = vec![evm::DelegationParam {
        chain_id: 0,
        address: authority,
        nonce: 0,
        y_parity: 0,
        r: vec![1u8; 31],
        s: vec![1u8; 33],
    }];
    let params = evm::ApplyAndCallParams {
        list,
        call: evm::ApplyCall { to: authority, value: vec![], input: vec![] },
    };
    rt.expect_validate_caller_any();
    let res = rt.call::<evm::EvmContractActor>(
        evm::Method::ApplyAndCall as u64,
        IpldBlock::serialize_dag_cbor(&params).unwrap(),
    );
    assert!(res.is_err());
    assert_eq!(res.err().unwrap().exit_code(), ExitCode::USR_ILLEGAL_ARGUMENT);
}

#[test]
fn apply_and_call_rejects_authority_preexistence_contract() {
    let mut rt = util::construct_and_verify(vec![]);
    const GAS_BASE_APPLY7702: i64 = 0;
    const GAS_PER_AUTH_TUPLE: i64 = 10_000;
    rt.expect_gas_charge(GAS_BASE_APPLY7702);
    rt.expect_gas_charge(GAS_PER_AUTH_TUPLE);

    // Override recover to control the recovered authority address.
    rt.recover_secp_pubkey_fn = Box::new(|_hash, _sig| {
        // Return a constant uncompressed pubkey (0x04 || 64 bytes)
        let mut pk = [0u8; 65];
        pk[0] = 0x04;
        for i in 1..65 {
            pk[i] = 0xAB;
        }
        Ok(pk)
    });
    // Compute the recovered Eth address from the above pubkey.
    use fil_actors_runtime::test_utils::hash as rt_hash;
    use fvm_shared::crypto::hash::SupportedHashes;
    let mut pk = [0u8; 65];
    pk[0] = 0x04;
    for i in 1..65 {
        pk[i] = 0xAB;
    }
    let (keccak64, _) = rt_hash(SupportedHashes::Keccak256, &pk[1..]);
    let mut recovered = [0u8; 20];
    recovered.copy_from_slice(&keccak64[12..32]);
    let recovered_eth = EthAddress(recovered);
    // Map recovered_eth (f4) to an ID with EVM code to trigger pre-existence rejection.
    use fil_actors_runtime::test_utils::EVM_ACTOR_CODE_ID;
    use fvm_shared::address::Address as FilAddress;
    let recovered_f4: FilAddress = recovered_eth.into();
    let recovered_id = FilAddress::new_id(0x555u64);
    rt.set_delegated_address(recovered_id.id().unwrap(), recovered_f4);
    rt.actor_code_cids.borrow_mut().insert(recovered_id, *EVM_ACTOR_CODE_ID);

    // Build tuple; the "address" field is the delegate pointer, not the authority.
    // Authority will be recovered to `recovered_eth` via the override above.
    let list = vec![evm::DelegationParam {
        chain_id: 0,
        address: recovered_eth,
        nonce: 0,
        y_parity: 0,
        r: vec![1u8; 32],
        s: vec![1u8; 32],
    }];
    let params = evm::ApplyAndCallParams {
        list,
        call: evm::ApplyCall { to: recovered_eth, value: vec![], input: vec![] },
    };
    rt.expect_validate_caller_any();
    let res = rt.call::<evm::EvmContractActor>(
        evm::Method::ApplyAndCall as u64,
        IpldBlock::serialize_dag_cbor(&params).unwrap(),
    );
    assert!(res.is_err());
    assert_eq!(res.err().unwrap().exit_code(), ExitCode::USR_ILLEGAL_ARGUMENT);
}
