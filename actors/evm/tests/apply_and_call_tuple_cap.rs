use fil_actor_evm as evm;
use fil_actors_evm_shared::address::EthAddress;
use fvm_ipld_encoding::ipld_block::IpldBlock;
use fvm_shared::error::ExitCode;

mod util;

#[test]
fn tuple_cap_allows_64() {
    let mut rt = util::construct_and_verify(vec![]);
    // Expect base + aggregated per-tuple charge (charged once for all tuples).
    // No gas expectations in tests (behavioral only).
    // Make signature recovery deterministic and unique per tuple by deriving
    // a synthetic uncompressed pubkey from the message hash.
    rt.recover_secp_pubkey_fn = Box::new(|hash, _sig| {
        let mut pk = [0u8; 65];
        pk[0] = 0x04;
        pk[1..33].copy_from_slice(&hash[..32]);
        pk[33..65].copy_from_slice(&hash[..32]);
        Ok(pk)
    });

    // Build 64 tuples at nonce=0; vary signature bytes to yield unique recovered authorities.
    let dst = EthAddress(hex_literal::hex!("bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"));
    let mut list = Vec::with_capacity(64);
    for i in 1..=64u8 {
        let varying_addr = EthAddress::from_id(1000 + i as u64);
        list.push(evm::DelegationParam {
            chain_id: 0,
            address: varying_addr,
            nonce: 0,
            y_parity: 0,
            r: vec![i; 32],
            s: vec![1u8; 32],
        });
    }
    let params = evm::ApplyAndCallParams {
        list,
        call: evm::ApplyCall { to: dst, value: vec![], input: vec![] },
    };
    rt.expect_validate_caller_any();
    let res = rt.call::<evm::EvmContractActor>(
        evm::Method::ApplyAndCall as u64,
        IpldBlock::serialize_dag_cbor(&params).unwrap(),
    );
    if let Err(e) = &res {
        println!("allows_64 error: code={} msg={}", e.exit_code().value(), e.msg());
    }
    assert!(res.is_ok());
}

#[test]
fn tuple_cap_rejects_65() {
    let mut rt = util::construct_and_verify(vec![]);
    // For >64 tuples, actor rejects early before charging intrinsic gas.
    // Provide a deterministic recovery function to avoid flakiness if this changes later.
    rt.recover_secp_pubkey_fn = Box::new(|hash, sig| {
        let mut pk = [0u8; 65];
        pk[0] = 0x04;
        pk[1..33].copy_from_slice(&hash[..32]);
        pk[33..65].fill(sig[0]);
        Ok(pk)
    });

    let dst = EthAddress(hex_literal::hex!("cccccccccccccccccccccccccccccccccccccccc"));
    let mut list = Vec::with_capacity(65);
    for n in 0..65u64 {
        list.push(evm::DelegationParam {
            chain_id: 0,
            address: dst,
            nonce: n,
            y_parity: 0,
            r: vec![1u8; 32],
            s: vec![1u8; 32],
        });
    }
    let params = evm::ApplyAndCallParams {
        list,
        call: evm::ApplyCall { to: dst, value: vec![], input: vec![] },
    };
    rt.expect_validate_caller_any();
    let res = rt.call::<evm::EvmContractActor>(
        evm::Method::ApplyAndCall as u64,
        IpldBlock::serialize_dag_cbor(&params).unwrap(),
    );
    assert!(res.is_err());
    assert_eq!(res.err().unwrap().exit_code(), ExitCode::USR_ILLEGAL_ARGUMENT);
}
