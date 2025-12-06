use fil_actor_ethaccount as ethaccount;
use fil_actors_evm_shared::address::EthAddress;
use fil_actors_evm_shared::eip7702;
use fil_actors_runtime::EAM_ACTOR_ID;
use fil_actors_runtime::test_utils::*;
use fvm_ipld_encoding::ipld_block::IpldBlock;
use fvm_shared::address::Address;
use hex_literal::hex;

#[test]
fn invalid_y_parity_and_lengths() {
    let mut rt = MockRuntime::new();
    rt.expect_validate_caller_any();
    // Receiver must be EthAccount; set a predictable f4 address under EAM namespace.
    rt.set_delegated_address(1000, Address::new_delegated(EAM_ACTOR_ID, &[0u8; 20]).unwrap());
    rt.caller.replace(Address::new_id(10));
    rt.receiver = Address::new_id(1000);

    // Construct params with invalid y_parity=2
    let list = vec![eip7702::DelegationParam {
        chain_id: 0,
        address: EthAddress([0u8; 20]),
        nonce: 0,
        y_parity: 2,
        r: vec![1u8; 32],
        s: vec![1u8; 32],
    }];
    let params = eip7702::ApplyAndCallParams {
        list,
        call: eip7702::ApplyCall { to: EthAddress([0u8; 20]), value: vec![], input: vec![] },
    };
    rt.expect_validate_caller_any();
    let res = rt.call::<ethaccount::EthAccountActor>(
        ethaccount::Method::ApplyAndCall as u64,
        IpldBlock::serialize_dag_cbor(&params).unwrap(),
    );
    assert!(res.is_err());

    // r length > 32
    let list = vec![eip7702::DelegationParam {
        chain_id: 0,
        address: EthAddress([0u8; 20]),
        nonce: 0,
        y_parity: 0,
        r: vec![1u8; 33],
        s: vec![1u8; 32],
    }];
    let params = eip7702::ApplyAndCallParams {
        list,
        call: eip7702::ApplyCall { to: EthAddress([0u8; 20]), value: vec![], input: vec![] },
    };
    rt.expect_validate_caller_any();
    let res = rt.call::<ethaccount::EthAccountActor>(
        ethaccount::Method::ApplyAndCall as u64,
        IpldBlock::serialize_dag_cbor(&params).unwrap(),
    );
    assert!(res.is_err());

    // s length > 32
    let list = vec![eip7702::DelegationParam {
        chain_id: 0,
        address: EthAddress([0u8; 20]),
        nonce: 0,
        y_parity: 0,
        r: vec![1u8; 32],
        s: vec![1u8; 33],
    }];
    let params = eip7702::ApplyAndCallParams {
        list,
        call: eip7702::ApplyCall { to: EthAddress([0u8; 20]), value: vec![], input: vec![] },
    };
    rt.expect_validate_caller_any();
    let res = rt.call::<ethaccount::EthAccountActor>(
        ethaccount::Method::ApplyAndCall as u64,
        IpldBlock::serialize_dag_cbor(&params).unwrap(),
    );
    assert!(res.is_err());

    // zero r/s
    let list = vec![eip7702::DelegationParam {
        chain_id: 0,
        address: EthAddress([0u8; 20]),
        nonce: 0,
        y_parity: 0,
        r: vec![0u8; 32],
        s: vec![0u8; 32],
    }];
    let params = eip7702::ApplyAndCallParams {
        list,
        call: eip7702::ApplyCall { to: EthAddress([0u8; 20]), value: vec![], input: vec![] },
    };
    rt.expect_validate_caller_any();
    let res = rt.call::<ethaccount::EthAccountActor>(
        ethaccount::Method::ApplyAndCall as u64,
        IpldBlock::serialize_dag_cbor(&params).unwrap(),
    );
    assert!(res.is_err());

    // r length 0
    let list = vec![eip7702::DelegationParam {
        chain_id: 0,
        address: EthAddress([0u8; 20]),
        nonce: 0,
        y_parity: 0,
        r: vec![],
        s: vec![1u8; 32],
    }];
    let params = eip7702::ApplyAndCallParams {
        list,
        call: eip7702::ApplyCall { to: EthAddress([0u8; 20]), value: vec![], input: vec![] },
    };
    rt.expect_validate_caller_any();
    let res = rt.call::<ethaccount::EthAccountActor>(
        ethaccount::Method::ApplyAndCall as u64,
        IpldBlock::serialize_dag_cbor(&params).unwrap(),
    );
    assert!(res.is_err());

    // s length 0
    let list = vec![eip7702::DelegationParam {
        chain_id: 0,
        address: EthAddress([0u8; 20]),
        nonce: 0,
        y_parity: 0,
        r: vec![1u8; 32],
        s: vec![],
    }];
    let params = eip7702::ApplyAndCallParams {
        list,
        call: eip7702::ApplyCall { to: EthAddress([0u8; 20]), value: vec![], input: vec![] },
    };
    rt.expect_validate_caller_any();
    let res = rt.call::<ethaccount::EthAccountActor>(
        ethaccount::Method::ApplyAndCall as u64,
        IpldBlock::serialize_dag_cbor(&params).unwrap(),
    );
    assert!(res.is_err());
}

#[test]
fn rejects_high_s_and_overflowing_s() {
    let ethaccount_id = 1000;
    let eth_f4 = Address::new_delegated(EAM_ACTOR_ID, &[0u8; 20]).unwrap();
    let call = eip7702::ApplyCall { to: EthAddress([0u8; 20]), value: vec![], input: vec![] };

    // High-S (N/2 + 1) should be rejected.
    {
        let mut rt = MockRuntime::new();
        rt.expect_validate_caller_any();
        rt.set_delegated_address(ethaccount_id, eth_f4);
        rt.caller.replace(Address::new_id(10));
        rt.receiver = Address::new_id(ethaccount_id);

        let list = vec![eip7702::DelegationParam {
            chain_id: 0,
            address: EthAddress([0u8; 20]),
            nonce: 0,
            y_parity: 0,
            r: vec![1u8; 32],
            s: hex!("7FFFFFFFFFFFFFFFFFFFFFFFFFFFFFFF5D576E7357A4501DDFE92F46681B20A1").to_vec(),
        }];
        let params = eip7702::ApplyAndCallParams { list, call: call.clone() };
        rt.expect_validate_caller_any();
        let res = rt.call::<ethaccount::EthAccountActor>(
            ethaccount::Method::ApplyAndCall as u64,
            IpldBlock::serialize_dag_cbor(&params).unwrap(),
        );
        assert!(res.is_err(), "ApplyAndCall should reject high-S signatures");
    }

    // S >= secp256k1 order should also be rejected.
    {
        let mut rt = MockRuntime::new();
        rt.expect_validate_caller_any();
        rt.set_delegated_address(ethaccount_id, eth_f4);
        rt.caller.replace(Address::new_id(10));
        rt.receiver = Address::new_id(ethaccount_id);

        let list = vec![eip7702::DelegationParam {
            chain_id: 0,
            address: EthAddress([0u8; 20]),
            nonce: 0,
            y_parity: 0,
            r: vec![1u8; 32],
            s: vec![0xFF; 32],
        }];
        let params = eip7702::ApplyAndCallParams { list, call: call.clone() };
        rt.expect_validate_caller_any();
        let res = rt.call::<ethaccount::EthAccountActor>(
            ethaccount::Method::ApplyAndCall as u64,
            IpldBlock::serialize_dag_cbor(&params).unwrap(),
        );
        assert!(res.is_err(), "ApplyAndCall should reject S >= curve order");
    }
}

// Nonce mismatch and duplicates covered in broader suites; focused invalid-length/yParity tests here.
