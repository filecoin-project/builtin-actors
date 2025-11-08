use fil_actor_ethaccount as ethaccount;
use fil_actors_evm_shared::address::EthAddress;
use fil_actors_evm_shared::eip7702;
use fil_actors_runtime::EAM_ACTOR_ID;
use fil_actors_runtime::test_utils::*;
use fvm_ipld_encoding::ipld_block::IpldBlock;
use fvm_shared::address::Address;

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
}

// Nonce mismatch and duplicates covered in broader suites; focused invalid-length/yParity tests here.
