use fil_actor_ethaccount as ethaccount;
use fil_actors_evm_shared::{address::EthAddress, eip7702};
use fil_actors_runtime::EAM_ACTOR_ID;
use fil_actors_runtime::test_utils::*;
use fvm_ipld_encoding::ipld_block::IpldBlock;
use fvm_shared::address::Address;

#[test]
fn reject_duplicate_authorities_receiver_only() {
    let mut rt = MockRuntime::new();
    rt.expect_validate_caller_any();
    rt.set_delegated_address(1000, Address::new_delegated(EAM_ACTOR_ID, &[0xAA; 20]).unwrap());
    rt.caller.replace(Address::new_id(10));
    rt.receiver = Address::new_id(1000);

    // Duplicate tuples (same authority) in a single message should be rejected.
    let auth = EthAddress([0xAB; 20]);
    let t = eip7702::DelegationParam {
        chain_id: 0,
        address: auth,
        nonce: 0,
        y_parity: 0,
        r: vec![1u8],
        s: vec![1u8],
    };
    let list = vec![t.clone(), t];
    let params = eip7702::ApplyAndCallParams {
        list,
        call: eip7702::ApplyCall { to: EthAddress([0u8; 20]), value: vec![], input: vec![] },
    };
    rt.expect_validate_caller_any();
    let res = rt.call::<ethaccount::EthAccountActor>(
        ethaccount::Method::ApplyAndCall as u64,
        IpldBlock::serialize_dag_cbor(&params).unwrap(),
    );
    assert!(res.is_err(), "duplicates must be rejected");
}
